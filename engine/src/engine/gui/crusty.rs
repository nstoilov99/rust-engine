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
use crusty_gui::input::{Event, Modifiers, RawInput};
use crusty_gui::math::{Color, Pos2, Rect, Rounding, Vec2};
use crusty_gui::paint::{PaintCmd, TextureFilter, TextureId};
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
/// ported panels visually match their egui counterparts. egui runs at
/// pixels_per_point 1.15 while crusty runs at 1.0, so point-sized tokens
/// (fonts, spacing) are pre-scaled here.
pub fn style_from_theme(theme: &EditorTheme) -> Style {
    const PPP: f32 = 1.15;
    let c = |c32: egui::Color32| Color::from_srgb_u8(c32.r(), c32.g(), c32.b(), c32.a());

    let p = &theme.palette;
    let sp = &theme.spacing;
    let ty = &theme.typography;

    let mut style = Style::editor_dark();

    style.palette.surface = c(p.field_bg);
    style.palette.surface_hover = c(p.surface[3]);
    style.palette.surface_active = c(p.surface[4]);
    style.palette.accent = c(p.accent);
    style.palette.accent_glow = c(p.accent).with_alpha(0.25);
    style.palette.text = c(p.text_primary);
    style.palette.text_dim = c(p.text_secondary);
    style.palette.stroke = c(p.stroke);
    style.palette.stroke_hover = c(p.accent);
    style.palette.success = c(p.semantic.success);

    style.spacing.item = sp.item_spacing_y * PPP;
    style.spacing.padding = sp.window_margin * PPP;
    style.spacing.button_padding =
        Vec2::new(sp.button_padding_x * PPP, sp.button_padding_y * PPP);
    style.spacing.box_label_gap = 6.0 * PPP;

    style.fonts.body = ty.body * PPP;
    style.fonts.title = ty.heading * PPP;
    style.fonts.small = ty.caption * PPP;

    style.rounding.panel = Rounding::same(6.0);
    style.rounding.widget = Rounding::same(3.0);
    style.rounding.small = Rounding::same(2.0);

    style.sizes.checkbox *= PPP;
    style.sizes.radio *= PPP;
    style.sizes.slider_height *= PPP;

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
        Self {
            ctx,
            text: Arc::new(Mutex::new(TextRenderer::new(device, [1024, 1024]))),
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
