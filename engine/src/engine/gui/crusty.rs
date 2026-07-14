//! crusty-gui integration mirroring the egui [`Gui`](super::Gui) seam.
//!
//! Phase 16 of the crusty-gui roadmap: the editor UI migrates from egui to
//! our in-house library panel by panel. The egui split is mirrored across
//! threads: [`CrustyGui`] lives on the main thread (event translation +
//! CPU-only layout, like `Gui::layout`), [`CrustyRenderer`] lives on the
//! render thread (records a command buffer targeting the swapchain image,
//! like the render thread's `EguiRenderer`). The paint list crosses in the
//! `FramePacket`; the glyph shaper/atlas is shared between both halves
//! behind a mutex (layout shapes text on the main thread, the renderer
//! flushes queued glyph uploads while recording).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crusty_gui::backend::TargetRenderer;
use crusty_gui::context::{Context, CursorIcon, Ui};
use crusty_gui::input::{Event, RawInput};
pub use crusty_gui::input::{Key, Modifiers, Shortcut};
use crusty_gui::math::{Color, Pos2, Rect, Rounding, Vec2};
use crusty_gui::paint::{PaintCmd, TextureFilter};
pub use crusty_gui::paint::TextureId;
use crusty_gui::shell::input as shell_input;
use crusty_gui::style::Style;
use crusty_gui::text::TextRenderer;

use crate::engine::editor::theme::EditorTheme;

pub use crusty_gui::shell::winit_cursor;

use vulkano::command_buffer::PrimaryAutoCommandBuffer;
use vulkano::command_buffer::allocator::{
    StandardCommandBufferAllocator, StandardCommandBufferAllocatorCreateInfo,
};
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::Image;
use vulkano::image::view::ImageView;
use winit::event::WindowEvent;

/// Glyph shaper/atlas shared between the main-thread layout pass and the
/// render-thread record pass.
pub type SharedTextRenderer = Arc<Mutex<TextRenderer>>;

/// Map the engine's [`EditorTheme`] tokens onto a crusty-gui [`Style`] so
/// ported panels visually match their egui counterparts. egui's effective
/// pixels_per_point is 1.0 at runtime (the 1.15 init in `Gui` is
/// overwritten by `full_output.pixels_per_point` each frame), so theme
/// point values map 1:1 to crusty's physical pixels — no pre-scaling.
pub fn style_from_theme(theme: &EditorTheme) -> Style {
    let c = |c32: egui::Color32| Color::from_srgb_u8(c32.r(), c32.g(), c32.b(), c32.a());

    let p = &theme.palette;
    let sp = &theme.spacing;
    let ty = &theme.typography;

    let mut style = Style::editor_dark();

    style.palette.surface = c(p.field_bg);
    style.palette.surface_hover = c(p.surface[3]);
    style.palette.surface_active = c(p.surface[4]);
    style.palette.accent = c(p.accent);
    // Zero alpha disables crusty's hover glow — egui hover is a plain 1px
    // accent stroke, no shadow.
    style.palette.accent_glow = c(p.accent).with_alpha(0.0);
    style.palette.text = c(p.text_primary);
    style.palette.text_dim = c(p.text_secondary);
    style.palette.stroke = c(p.stroke);
    style.palette.stroke_hover = c(p.accent);
    style.palette.success = c(p.semantic.success);
    // Dock chrome: panel bodies on surface[1] (egui's panel bg), tab strip
    // on the darker surface[0] so the active tab connects to its body.
    style.palette.panel = c(p.surface[1]);
    style.palette.tab_bar = c(p.surface[0]);

    style.spacing.item = sp.item_spacing_y;
    style.spacing.padding = sp.window_margin;
    style.spacing.button_padding = Vec2::new(sp.button_padding_x, sp.button_padding_y);
    style.spacing.box_label_gap = 6.0;

    style.fonts.body = ty.body;
    // egui's `ui.heading` maps TextStyle::Heading to `heading_large`.
    style.fonts.title = ty.heading_large;
    style.fonts.small = ty.caption;

    style.rounding.panel = Rounding::same(6.0);
    style.rounding.widget = Rounding::same(3.0);
    style.rounding.small = Rounding::same(2.0);

    style
}

/// Result of the CPU-only layout pass — the crusty-gui analogue of
/// [`GuiLayoutResult`](super::GuiLayoutResult). The paint list crosses to
/// the render thread; the flags gate game input on the main thread.
pub struct CrustyLayoutResult {
    pub paint: Vec<PaintCmd>,
    pub screen_size: [f32; 2],
    pub wants_keyboard: bool,
    pub wants_pointer: bool,
    /// Pointer is held by a crusty floating layer / pressed widget / drag.
    /// While set, pointer events must be withheld from egui too, or clicks
    /// on floating crusty popups leak into the dock beneath them.
    pub owns_pointer: bool,
    pub cursor_icon: CursorIcon,
    /// `Some(ZERO)` = animating, repaint now; `None` = idle until input.
    pub repaint_after: Option<Duration>,
}

/// Main-thread half: UI context, input translation and layout.
pub struct CrustyGui {
    ctx: Context,
    text: SharedTextRenderer,
    screen_size: [f32; 2],

    pointer_pos: Option<Pos2>,
    modifiers: Modifiers,
    events: Vec<Event>,
    start: Instant,
    last_frame: Instant,

    dropped_files: Vec<PathBuf>,
    hovered_file_count: usize,
}

impl CrustyGui {
    pub fn new(device: Arc<Device>, screen_size: [f32; 2]) -> Self {
        let mut ctx = Context::new();
        ctx.style = style_from_theme(&EditorTheme::dark_default());
        let mut text = TextRenderer::new(device, [1024, 1024]);
        // Shape with the same font egui uses (its bundled default,
        // Ubuntu-Light) so ported panels match the egui ones glyph-for-glyph
        // instead of falling back to the OS default font.
        let fonts = egui::FontDefinitions::default();
        if let Some(data) = fonts.font_data.get("Ubuntu-Light") {
            if let Some(family) = text.load_font(data.font.to_vec()) {
                text.set_default_family(family);
            }
        }
        Self {
            ctx,
            text: Arc::new(Mutex::new(text)),
            screen_size,
            pointer_pos: None,
            modifiers: Modifiers::empty(),
            events: Vec::new(),
            start: Instant::now(),
            last_frame: Instant::now(),
            dropped_files: Vec::new(),
            hovered_file_count: 0,
        }
    }

    /// Handle for the render thread's [`CrustyRenderer`].
    pub fn text_handle(&self) -> SharedTextRenderer {
        self.text.clone()
    }

    /// Translate a winit event into queued UI input. Returns true when the
    /// event was UI-relevant (same contract as `Gui::handle_event`).
    pub fn handle_event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::DroppedFile(path) => {
                self.dropped_files.push(path.clone());
                self.hovered_file_count = 0;
                true
            }
            WindowEvent::HoveredFile(_) => {
                self.hovered_file_count += 1;
                true
            }
            WindowEvent::HoveredFileCancelled => {
                self.hovered_file_count = 0;
                true
            }
            WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::KeyboardInput { .. } => {
                shell_input::apply_input_event(
                    event.clone(),
                    &mut self.pointer_pos,
                    &mut self.events,
                    &mut self.modifiers,
                );
                true
            }
            _ => false,
        }
    }

    /// CPU-only UI pass: runs the context frame and returns the paint list
    /// plus input-gating flags. No GPU commands are recorded.
    pub fn layout(&mut self, ui_fn: impl FnOnce(&mut Ui)) -> CrustyLayoutResult {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().clamp(1.0 / 1000.0, 0.1);
        self.last_frame = now;

        let screen_rect = Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(self.screen_size[0], self.screen_size[1]),
        );
        let input = RawInput {
            screen_rect,
            pixels_per_point: 1.0,
            time: self.start.elapsed().as_secs_f64(),
            predicted_dt: dt,
            modifiers: self.modifiers,
            events: std::mem::take(&mut self.events),
        };

        self.ctx.begin_frame(input);
        {
            let mut text = self.text.lock();
            self.ctx.run_root(&mut text, screen_rect, ui_fn);
        }
        let output = self.ctx.end_frame();

        CrustyLayoutResult {
            paint: output.paint,
            screen_size: self.screen_size,
            wants_keyboard: output.wants_keyboard,
            wants_pointer: output.wants_pointer,
            owns_pointer: output.owns_pointer,
            cursor_icon: output.cursor_icon,
            repaint_after: output.repaint_after,
        }
    }

    /// Call when the window resizes. Zero sizes (minimized) are ignored.
    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        if width > 0.0 && height > 0.0 {
            self.screen_size = [width, height];
        }
    }

    /// Drain files dropped from the OS this frame.
    pub fn take_dropped_files(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.dropped_files)
    }

    /// True while the OS hovers files over the window (before drop).
    pub fn is_hovering_external_files(&self) -> bool {
        self.hovered_file_count > 0
    }

    /// Re-derive the crusty style from the engine theme (call after theme
    /// or density changes so both UIs stay in sync).
    pub fn apply_theme(&mut self, theme: &EditorTheme) {
        self.ctx.style = style_from_theme(theme);
    }

    /// Direct access to the UI context (style, memory, repaint requests).
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.ctx
    }
}

/// Render-thread half: records the UI paint list into the swapchain image.
pub struct CrustyRenderer {
    renderer: TargetRenderer,
    text: SharedTextRenderer,
}

impl CrustyRenderer {
    /// `format` must match the images later passed to [`render`](Self::render).
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        format: Format,
        text: SharedTextRenderer,
    ) -> Self {
        let cmd_alloc = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            StandardCommandBufferAllocatorCreateInfo::default(),
        ));
        Self {
            renderer: TargetRenderer::new(device, queue, cmd_alloc, format),
            text,
        }
    }

    /// Record the UI paint list into `target` (typically the swapchain
    /// image, after the scene / egui pass) and return the command buffer.
    /// `backdrop` is what glass panels blur; `clear` clears the target
    /// first instead of compositing over it.
    ///
    /// Returns `Ok(None)` when the target has a zero extent (minimized).
    pub fn render(
        &mut self,
        target: Arc<Image>,
        paint: &[PaintCmd],
        backdrop: Option<Arc<ImageView>>,
        clear: Option<[f32; 4]>,
    ) -> Result<Option<Arc<PrimaryAutoCommandBuffer>>, Box<dyn std::error::Error>> {
        let view = ImageView::new_default(target)?;
        let mut text = self.text.lock();
        Ok(self
            .renderer
            .render(&mut text, view, paint, backdrop, clear))
    }

    /// Register an engine image view (viewport render target, thumbnail) so
    /// the UI can draw it with the `Image` widget.
    pub fn register_native_texture(&mut self, view: Arc<ImageView>) -> TextureId {
        self.renderer.registry_mut().register(view, TextureFilter::Linear)
    }

    /// Upload raw RGBA8 pixels to a new GPU image and register it for UI
    /// drawing (asset-browser thumbnails, icons).
    pub fn upload_rgba_texture(
        &mut self,
        queue: Arc<Queue>,
        memory_allocator: Arc<vulkano::memory::allocator::StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<TextureId, Box<dyn std::error::Error>> {
        use vulkano::image::{ImageCreateInfo, ImageType, ImageUsage};
        use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter};

        let image = Image::new(
            memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_SRGB,
                extent: [width, height, 1],
                usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )?;
        crate::engine::assets::texture::upload_texture_data(
            command_buffer_allocator,
            queue,
            memory_allocator,
            image.clone(),
            rgba,
        )?;
        let view = ImageView::new_default(image)?;
        Ok(self.register_native_texture(view))
    }

    /// Rasterize the hierarchy panel's SVG icon set, upload each to a GPU
    /// image and register it, returning `file stem → TextureId`. Called
    /// once at render-thread startup; the map crosses to the main thread in
    /// `RenderEvent::RenderThreadReady` so the crusty layout pass can draw
    /// the icons. Missing dir / broken SVGs are logged and skipped — the
    /// panel falls back to a dot glyph for absent stems.
    pub fn load_hierarchy_icons(
        &mut self,
        queue: Arc<Queue>,
        memory_allocator: Arc<vulkano::memory::allocator::StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
    ) -> std::collections::HashMap<String, TextureId> {
        use crate::engine::editor::hierarchy_icons::{icon_raster_px, icons_dir, rasterize_svg_rgba};

        let mut map = std::collections::HashMap::new();
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        match std::fs::read_dir(icons_dir()) {
            Ok(e) => paths.extend(e.flatten().map(|entry| entry.path())),
            Err(e) => {
                log::warn!("crusty: hierarchy icons dir unreadable ({}): {e}", icons_dir().display());
            }
        }
        // Extra editor icons living one level up from the hierarchy set.
        if let Some(parent) = icons_dir().parent() {
            for name in [
                "color-picker.svg",
                "folder.svg",
                "opened-folder.svg",
                "folder-browser.svg",
                "grid-view.svg",
                "list-view.svg",
                "add.svg",
                // Viewport toolbar set.
                "select.svg",
                "translate.svg",
                "rotate.png",
                "scale.svg",
                "world.svg",
                "local.svg",
                "grid-snap.svg",
                "rotation-snap.svg",
                "scale-snap.svg",
                "camera-speed.svg",
                "file-mesh.png",
                "image-file.png",
                "file-document.png",
                "code-file.png",
                // Menu bar play controls.
                "play-fill.png",
                "pause-fill.png",
                "stop-fill.png",
                "skip-forward-fill.png",
                // Mesh editor material slots.
                "save-check.svg",
                "save-changes.svg",
                "trash.svg",
                "undo.svg",
                "tree-view.svg",
                "add-circle.svg",
            ] {
                paths.push(parent.join(name));
            }
        }
        let px = icon_raster_px();

        for path in paths {
            let ext = path.extension().and_then(|e| e.to_str());
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string)
            else {
                continue;
            };
            let (rgba, w, h) = match ext {
                Some("svg") => match rasterize_svg_rgba(&path) {
                    Ok(d) => (d, px, px),
                    Err(e) => {
                        log::warn!("crusty: failed to rasterize {}: {e}", path.display());
                        continue;
                    }
                },
                Some("png") => {
                    match std::fs::read(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|b| image::load_from_memory(&b).map_err(|e| e.to_string()))
                    {
                        Ok(img) => {
                            let img = img.to_rgba8();
                            let (w, h) = img.dimensions();
                            (img.into_raw(), w, h)
                        }
                        Err(e) => {
                            log::warn!("crusty: failed to load {}: {e}", path.display());
                            continue;
                        }
                    }
                }
                _ => continue,
            };
            match self.upload_rgba_texture(
                queue.clone(),
                memory_allocator.clone(),
                command_buffer_allocator.clone(),
                &rgba,
                w,
                h,
            ) {
                Ok(id) => {
                    map.insert(stem, id);
                }
                Err(e) => log::warn!("crusty: icon upload failed for {stem}: {e}"),
            }
        }
        map
    }

    /// Point an existing texture id at a new image view (viewport resize).
    pub fn update_native_texture(&mut self, id: TextureId, view: Arc<ImageView>) {
        self.renderer.registry_mut().update(id, view);
    }

    /// Drop a registered texture.
    pub fn remove_native_texture(&mut self, id: TextureId) {
        self.renderer.registry_mut().remove(id);
    }

    /// Call on swapchain recreation so cached framebuffers don't pin the
    /// old images.
    pub fn clear_framebuffer_cache(&mut self) {
        self.renderer.clear_target_cache();
    }
}
