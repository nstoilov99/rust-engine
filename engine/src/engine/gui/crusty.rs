//! crusty-gui integration mirroring the egui [`Gui`](super::Gui) seam.
//!
//! Phase 16 of the crusty-gui roadmap: the editor UI migrates from egui to
//! our in-house library panel by panel. This struct is the crusty-gui
//! counterpart of `Gui` — same split between a CPU-only [`layout`] pass
//! (main thread) and a [`render`] pass that records a command buffer for
//! the frame graph (render thread), same winit event translation entry
//! point, same native-texture registration for the viewport.
//!
//! [`layout`]: CrustyGui::layout
//! [`render`]: CrustyGui::render

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crusty_gui::backend::TargetRenderer;
use crusty_gui::context::{Context, CursorIcon, Ui};
use crusty_gui::input::{Event, Modifiers, RawInput};
use crusty_gui::math::{Pos2, Rect, Vec2};
use crusty_gui::paint::{PaintCmd, TextureFilter, TextureId};
use crusty_gui::shell::input as shell_input;
use crusty_gui::text::TextRenderer;

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

/// crusty-gui equivalent of [`Gui`](super::Gui): owns the UI context, the
/// glyph shaper/atlas and the presentation-less renderer.
pub struct CrustyGui {
    ctx: Context,
    text: TextRenderer,
    renderer: TargetRenderer,
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
    /// `format` must match the images later passed to [`render`](Self::render).
    pub fn new(
        device: Arc<Device>,
        queue: Arc<Queue>,
        format: Format,
        screen_size: [f32; 2],
    ) -> Self {
        let cmd_alloc = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            StandardCommandBufferAllocatorCreateInfo::default(),
        ));
        let text = TextRenderer::new(device.clone(), [1024, 1024]);
        let renderer = TargetRenderer::new(device, queue, cmd_alloc, format);

        Self {
            ctx: Context::new(),
            text,
            renderer,
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
        self.ctx.run_root(&mut self.text, screen_rect, ui_fn);
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

    /// Record the UI paint list into `target` (typically the swapchain
    /// image, after the scene) and return the command buffer for the frame
    /// graph. `backdrop` is what glass panels blur (the scene image);
    /// `clear` clears the target first instead of compositing.
    ///
    /// Returns `None` when the target has a zero extent (minimized).
    pub fn render(
        &mut self,
        target: Arc<Image>,
        paint: &[PaintCmd],
        backdrop: Option<Arc<ImageView>>,
        clear: Option<[f32; 4]>,
    ) -> Result<Option<Arc<PrimaryAutoCommandBuffer>>, Box<dyn std::error::Error>> {
        let view = ImageView::new_default(target)?;
        Ok(self
            .renderer
            .render(&mut self.text, view, paint, backdrop, clear))
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

    /// Call when the window resizes. Zero sizes (minimized) are ignored.
    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        if width > 0.0 && height > 0.0 {
            self.screen_size = [width, height];
        }
    }

    /// Call on swapchain recreation so cached framebuffers don't pin the
    /// old images.
    pub fn clear_framebuffer_cache(&mut self) {
        self.renderer.clear_target_cache();
    }

    /// Drain files dropped from the OS this frame.
    pub fn take_dropped_files(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.dropped_files)
    }

    /// True while the OS hovers files over the window (before drop).
    pub fn is_hovering_external_files(&self) -> bool {
        self.hovered_file_count > 0
    }

    /// Direct access to the UI context (style, memory, repaint requests).
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.ctx
    }
}
