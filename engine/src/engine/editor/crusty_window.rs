//! Floating OS windows for torn-off crusty dock tabs.
//!
//! Each float window owns its own winit Window, Vulkan Surface/Swapchain and
//! a private `CrustyGui`/`CrustyRenderer` pair (own glyph atlas + texture
//! registry), rendered on the main thread. GPU device/queue are shared with
//! the main renderer — vulkano queues are internally synchronized, and a
//! private `TargetRenderer` per window keeps the serial-execution contract.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crusty_gui::context::Ui;
use crusty_gui::dock::{DockNode, DockState};
use crusty_gui::math::{Color, Pos2, Rect, Rounding, Vec2};
use crusty_gui::paint::{PaintCmd, RectShape, Stroke, TextureId};
use vulkano::command_buffer::allocator::StandardCommandBufferAllocator;
use vulkano::command_buffer::PrimaryAutoCommandBuffer;
use vulkano::device::{Device, Queue};
use vulkano::image::view::ImageView;
use vulkano::image::Image;
use vulkano::memory::allocator::StandardMemoryAllocator;
use vulkano::swapchain::{self, Surface, Swapchain, SwapchainPresentInfo};
use vulkano::sync::future::FenceSignalFuture;
use vulkano::sync::GpuFuture;
use vulkano::{Validated, VulkanError};
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::window::{CursorIcon, ResizeDirection, Window};

use super::dock_crusty::{self, DockArea};
use super::dock_layout::EditorTab;
use crate::engine::core::swapchain::{create_swapchain, recreate_swapchain};
use crate::engine::gui::crusty::{CrustyGui, CrustyRenderer};

/// Request to spawn a float window for a torn-off tab, produced by the main
/// dock when a drag is released over no drop target.
pub struct CrustyWindowRequest {
    /// Crusty dock tab id (see `dock_crusty::tab_id`).
    pub tab: String,
    /// Cursor position in the *main window's* client space at release, so
    /// the new window can appear under the cursor.
    pub main_local: Pos2,
}

/// A tab picked up from a float window: the whole window follows the cursor
/// until release (cross-window re-dock drag).
pub struct DragOut {
    pub tab: String,
    /// Cursor offset within the window's client area at pickup.
    pub grab: Vec2,
}

pub struct CrustyFloatWindow {
    pub window: Arc<Window>,
    surface: Arc<Surface>,
    swapchain: Arc<Swapchain>,
    images: Vec<Arc<Image>>,
    recreate_swapchain: bool,
    previous_frame_end: Option<FenceSignalFuture<Box<dyn GpuFuture>>>,
    pub gui: CrustyGui,
    renderer: CrustyRenderer,
    /// This window's own icon uploads (TextureIds are registry-local).
    pub icons: HashMap<String, TextureId>,
    /// Mesh-preview render targets registered with this window's own
    /// renderer (ids are registry-local). Key = mesh editor key.
    pub mesh_textures: HashMap<String, (TextureId, Arc<ImageView>)>,
    /// Material thumbnails uploaded into this window's registry (the main
    /// window's thumbnail ids are invalid here).
    pub thumb_ids: HashMap<crate::engine::assets::AssetId, TextureId>,
    pub tree: DockNode,
    pub state: DockState,
    pub close_requested: bool,
    pub drag_out: Option<DragOut>,
    /// Last client-space cursor position.
    cursor: Pos2,
    /// Left button released since the last frame.
    pub released: bool,
    /// Borderless-window edge under the cursor (OS resize hit test).
    resize_zone: Option<ResizeDirection>,
}

/// Height of the caption strip = the dock tab bar (window is borderless,
/// the tab bar doubles as the title bar, Unreal-style).
const CAPTION_H: f32 = 28.0;
const CAPTION_BTN_W: f32 = 34.0;
const RESIZE_BORDER: f32 = 6.0;
const RESIZE_CORNER: f32 = 14.0;

/// Clicks on the caption controls, applied by the host after layout.
#[derive(Default)]
struct CaptionAction {
    minimize: bool,
    toggle_maximize: bool,
    close: bool,
    /// Press on empty caption space — start an OS window move.
    drag: bool,
}

impl CrustyFloatWindow {
    pub fn new(
        window: Arc<Window>,
        device: Arc<Device>,
        queue: Arc<Queue>,
        memory_allocator: Arc<StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
        tab: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let instance = device.physical_device().instance().clone();
        let surface = Surface::from_window(instance, window.clone())?;
        let (swapchain, images) = create_swapchain(device.clone(), surface.clone())?;
        let format = images[0].format();

        let size = window.inner_size();
        let gui = CrustyGui::new(device.clone(), [size.width as f32, size.height as f32]);
        let mut renderer = CrustyRenderer::new(device, queue.clone(), format, gui.text_handle());
        let icons =
            renderer.load_hierarchy_icons(queue, memory_allocator, command_buffer_allocator);

        Ok(Self {
            window,
            surface,
            swapchain,
            images,
            recreate_swapchain: false,
            previous_frame_end: None,
            gui,
            renderer,
            icons,
            mesh_textures: HashMap::new(),
            thumb_ids: HashMap::new(),
            tree: DockNode::leaf(tab),
            state: DockState::default(),
            close_requested: false,
            drag_out: None,
            cursor: Pos2::ZERO,
            released: false,
            resize_zone: None,
        })
    }

    /// Route a winit event for this window. Tracks the raw cursor/button for
    /// the cross-window drag before forwarding to the UI input translation.
    pub fn handle_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.close_requested = true,
            WindowEvent::Resized(size) => {
                self.recreate_swapchain = true;
                self.gui.set_screen_size(size.width as f32, size.height as f32);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Pos2::new(position.x as f32, position.y as f32);
                self.update_resize_zone();
                self.gui.handle_event(event);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if self.resize_zone.is_some() && self.drag_out.is_none() => {
                // Borderless edge: hand the resize to the OS, keep the press
                // away from the UI.
                if let Some(dir) = self.resize_zone {
                    let _ = self.window.drag_resize_window(dir);
                }
            }
            _ => {
                if let WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } = event
                {
                    self.released = true;
                }
                self.gui.handle_event(event);
            }
        }
    }

    /// Edge/corner hit test for the borderless window; updates the OS cursor
    /// when the zone changes.
    fn update_resize_zone(&mut self) {
        let zone = if self.window.is_maximized() {
            None
        } else {
            let size = self.window.inner_size();
            let (w, h) = (size.width as f32, size.height as f32);
            let (x, y) = (self.cursor.x, self.cursor.y);
            let (l, r) = (x < RESIZE_BORDER, x > w - RESIZE_BORDER);
            let (t, b) = (y < RESIZE_BORDER, y > h - RESIZE_BORDER);
            // Corners get a wider grab area along the adjacent edges.
            let (cl, cr) = (x < RESIZE_CORNER, x > w - RESIZE_CORNER);
            let (ct, cb) = (y < RESIZE_CORNER, y > h - RESIZE_CORNER);
            if (t && cl) || (l && ct) {
                Some(ResizeDirection::NorthWest)
            } else if (t && cr) || (r && ct) {
                Some(ResizeDirection::NorthEast)
            } else if (b && cl) || (l && cb) {
                Some(ResizeDirection::SouthWest)
            } else if (b && cr) || (r && cb) {
                Some(ResizeDirection::SouthEast)
            } else if l {
                Some(ResizeDirection::West)
            } else if r {
                Some(ResizeDirection::East)
            } else if t {
                Some(ResizeDirection::North)
            } else if b {
                Some(ResizeDirection::South)
            } else {
                None
            }
        };
        if zone != self.resize_zone {
            self.window.set_cursor(match zone {
                Some(ResizeDirection::West) | Some(ResizeDirection::East) => CursorIcon::EwResize,
                Some(ResizeDirection::North) | Some(ResizeDirection::South) => {
                    CursorIcon::NsResize
                }
                Some(ResizeDirection::NorthWest) | Some(ResizeDirection::SouthEast) => {
                    CursorIcon::NwseResize
                }
                Some(ResizeDirection::NorthEast) | Some(ResizeDirection::SouthWest) => {
                    CursorIcon::NeswResize
                }
                None => CursorIcon::Default,
            });
            self.resize_zone = zone;
        }
    }

    /// Current drag cursor in screen coordinates (client cursor + window
    /// client origin).
    pub fn drag_screen_pos(&self) -> Option<Pos2> {
        let origin = self.window.inner_position().ok()?;
        Some(Pos2::new(
            origin.x as f32 + self.cursor.x,
            origin.y as f32 + self.cursor.y,
        ))
    }

    /// Register or re-point a mesh-preview render target with this window's
    /// private texture registry. Returns the registry-local id.
    ///
    /// `has_cb` = this window executes the key's preview render CB this
    /// frame. A view is only (re)bound when that CB is chained in the same
    /// submission — a freshly created/resized target is still in Undefined
    /// layout, and sampling it without its render CB is a validation panic.
    /// Without the CB we keep sampling the previous view (valid layout).
    pub fn ensure_mesh_texture(
        &mut self,
        key: &str,
        view: Arc<ImageView>,
        has_cb: bool,
    ) -> Option<TextureId> {
        match self.mesh_textures.get_mut(key) {
            Some((id, stored)) => {
                if !Arc::ptr_eq(stored, &view) && has_cb {
                    self.renderer.update_native_texture(*id, view.clone());
                    *stored = view;
                }
                Some(*id)
            }
            None if has_cb => {
                let id = self.renderer.register_native_texture(view.clone());
                self.mesh_textures.insert(key.to_string(), (id, view));
                Some(id)
            }
            None => None,
        }
    }

    /// Upload a material thumbnail into this window's registry (no-op if the
    /// asset is already registered). Pixel data comes from the shared
    /// `ThumbnailCache`'s retained rgba.
    pub fn register_thumb(
        &mut self,
        queue: Arc<Queue>,
        memory_allocator: Arc<StandardMemoryAllocator>,
        command_buffer_allocator: Arc<StandardCommandBufferAllocator>,
        id: crate::engine::assets::AssetId,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) {
        if self.thumb_ids.contains_key(&id) {
            return;
        }
        match self.renderer.upload_rgba_texture(
            queue,
            memory_allocator,
            command_buffer_allocator,
            rgba,
            width,
            height,
        ) {
            Ok(tex) => {
                self.thumb_ids.insert(id, tex);
            }
            Err(e) => log::warn!("crusty float: thumbnail upload failed: {e}"),
        }
    }

    /// Drop mesh-preview textures whose editor key no longer passes `keep`
    /// (tab re-docked or closed), releasing the registry slots.
    pub fn prune_mesh_textures(&mut self, keep: impl Fn(&str) -> bool) {
        let dead: Vec<String> = self
            .mesh_textures
            .keys()
            .filter(|k| !keep(k))
            .cloned()
            .collect();
        for k in dead {
            if let Some((id, _)) = self.mesh_textures.remove(&k) {
                self.renderer.remove_native_texture(id);
            }
        }
    }

    /// Layout + render one frame. `panel_fn` draws a tab body exactly like
    /// the main dock's closure; it receives this window's own `icons` map
    /// (TextureIds are registry-local, the main window's ids don't apply).
    /// `preview_cbs` are pre-recorded mesh-preview renders executed before
    /// the GUI pass so the sampled textures are ready in the same submission.
    pub fn frame(
        &mut self,
        device: Arc<Device>,
        queue: Arc<Queue>,
        titles: &HashMap<String, String>,
        preview_cbs: Vec<Arc<PrimaryAutoCommandBuffer>>,
        mut panel_fn: impl FnMut(
            &mut crusty_gui::context::Ui,
            &str,
            &HashMap<String, TextureId>,
            &HashMap<crate::engine::assets::AssetId, TextureId>,
        ),
    ) -> Result<(), Box<dyn std::error::Error>> {
        let size = self.window.inner_size();
        let mut tabs = Vec::new();
        self.tree.collect_tabs(&mut tabs);
        if size.width == 0 || size.height == 0 || tabs.is_empty() {
            return Ok(());
        }
        self.gui.set_screen_size(size.width as f32, size.height as f32);

        // While a tab is being dragged out, the whole window follows the
        // cursor (the OS window itself is the drag ghost).
        if let Some(drag) = &self.drag_out {
            if let Ok(outer) = self.window.outer_position() {
                // Keep the grab point under the cursor: shift outer by the
                // difference between the desired and current client origin.
                let dx = self.cursor.x - drag.grab.x;
                let dy = self.cursor.y - drag.grab.y;
                if dx.abs() > 0.5 || dy.abs() > 0.5 {
                    self.window.set_outer_position(PhysicalPosition::new(
                        outer.x + dx as i32,
                        outer.y + dy as i32,
                    ));
                }
            }
        }

        // ── Layout (CPU).
        let maximized = self.window.is_maximized();
        let Self {
            gui,
            tree,
            state,
            icons,
            thumb_ids,
            ..
        } = self;
        let rect = Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(size.width as f32, size.height as f32),
        );
        let mut detached = None;
        let mut close_tab = None;
        let mut caption = CaptionAction::default();
        let result = gui.layout(|ui| {
            let resp = DockArea::new(tree, state)
                .titles(titles)
                .lone_tearoff(true)
                .show_close_buttons(true)
                .min_tab_width(120.0)
                .tab_bar_height(CAPTION_H)
                .show_in(ui, rect, |ui, tab| panel_fn(ui, tab, icons, thumb_ids));
            detached = resp.detached;
            close_tab = resp.close_requested;
            let tabs_end_x = resp.tab_bar_slots.first().map_or(rect.min.x, |s| s.end_x);
            caption = caption_controls(ui, rect, tabs_end_x, maximized);
        });
        if let Some(tab) = close_tab {
            // Non-viewport panels are always closable; an empty window is
            // removed by the host next frame.
            self.tree.close_tab(&tab);
        }
        if let Some(det) = detached {
            // Keep rendering the panel — the window itself is the ghost.
            self.tree.add_tab(det.tab.clone());
            self.drag_out = Some(DragOut {
                tab: det.tab,
                grab: Vec2::new(self.cursor.x.clamp(20.0, 160.0), 14.0),
            });
        }
        if caption.close {
            self.close_requested = true;
        }
        if caption.minimize {
            self.window.set_minimized(true);
        }
        if caption.toggle_maximize {
            self.window.set_maximized(!maximized);
        }
        if caption.drag && self.drag_out.is_none() {
            let _ = self.window.drag_window();
        }
        // ── Render (same submit/present pattern as SecondaryWindow).
        // At most one float frame in flight: frames here are pumped from
        // OS modal move/resize loops, where an unbounded backlog of pending
        // presents on the shared queue can wedge a later flush for good.
        if let Some(prev) = self.previous_frame_end.take() {
            if prev.wait(Some(Duration::from_secs(1))).is_err() {
                self.previous_frame_end = Some(prev);
                return Ok(());
            }
        }
        if self.recreate_swapchain {
            match recreate_swapchain(device.clone(), self.surface.clone(), self.swapchain.clone())
            {
                Ok((new_swapchain, new_images)) => {
                    if new_images.is_empty() {
                        return Ok(());
                    }
                    self.swapchain = new_swapchain;
                    self.images = new_images;
                    self.recreate_swapchain = false;
                }
                Err(e) => {
                    log::warn!("crusty float window swapchain recreation failed: {e}");
                    return Ok(());
                }
            }
        }

        let (image_index, suboptimal, acquire_future) = match swapchain::acquire_next_image(
            self.swapchain.clone(),
            Some(Duration::from_millis(500)),
        ) {
            Ok(r) => r,
            Err(Validated::Error(VulkanError::OutOfDate | VulkanError::Timeout)) => {
                self.recreate_swapchain = true;
                return Ok(());
            }
            Err(e) => {
                log::warn!("crusty float window acquire failed: {e:?}");
                self.recreate_swapchain = true;
                return Ok(());
            }
        };
        if suboptimal {
            self.recreate_swapchain = true;
        }

        let target = self.images[image_index as usize].clone();
        // Clear to the theme's window background (linear ≈ srgb 18,18,21).
        let Some(cb) = self
            .renderer
            .render(target, &result.paint, None, Some([0.0056, 0.0056, 0.0075, 1.0]))?
        else {
            return Ok(());
        };

        let mut pre: Box<dyn GpuFuture> = acquire_future.boxed();
        for pcb in preview_cbs {
            pre = pre
                .then_execute(queue.clone(), pcb)
                .map_err(|e| format!("crusty float preview execute failed: {e:?}"))?
                .boxed();
        }
        let future = pre
            .then_execute(queue.clone(), cb)
            .map_err(|e| format!("crusty float window execute failed: {e:?}"))?;
        let _submit_guard =
            crate::engine::rendering::common::gpu_context::lock_queue_submit();
        if let Err(e) = future.flush() {
            log::error!("crusty float window flush failed: {e:?}");
            // SAFETY: flush attempted — mark finished so the CBEF drop can't
            // re-submit a OneTimeSubmit command buffer.
            unsafe { future.signal_finished() };
            queue.with(|mut q| q.wait_idle()).ok();
            return Ok(());
        }
        // SAFETY: flush succeeded — all CBs submitted.
        unsafe { future.signal_finished() };

        let present = future
            .then_swapchain_present(
                queue.clone(),
                SwapchainPresentInfo::swapchain_image_index(
                    self.swapchain.clone(),
                    image_index,
                ),
            )
            .boxed()
            .then_signal_fence_and_flush();
        match present {
            Ok(f) => self.previous_frame_end = Some(f),
            Err(e) => {
                log::warn!("crusty float window present failed: {e:?}");
                self.recreate_swapchain = true;
                queue.with(|mut q| q.wait_idle()).ok();
            }
        }
        Ok(())
    }
}

/// Draw minimize / maximize / close buttons in the caption's top-right corner
/// and detect a press on empty caption space (window move). Runs inside the
/// layout closure, painting over the dock tab bar.
fn caption_controls(ui: &mut Ui, win: Rect, tabs_end_x: f32, maximized: bool) -> CaptionAction {
    let ctx = ui.ctx_mut();
    let mut act = CaptionAction::default();
    let bar = Rect::from_min_size(win.min, Vec2::new(win.width(), CAPTION_H));
    let buttons_x = bar.max.x - 3.0 * CAPTION_BTN_W;

    for i in 0..3usize {
        let rect = Rect::from_min_size(
            Pos2::new(buttons_x + i as f32 * CAPTION_BTN_W, bar.min.y),
            Vec2::new(CAPTION_BTN_W, CAPTION_H),
        );
        let hovered = ctx.pointer_over(None, rect);
        if hovered {
            let fill = if i == 2 {
                // Windows-convention red close hover.
                Color::from_srgb_u8(196, 43, 28, 255)
            } else {
                ctx.style.palette.surface_hover
            };
            ctx.paint
                .push(PaintCmd::Rrect(RectShape::fill(rect, 0.0, fill)));
        }
        let col = if hovered {
            ctx.style.palette.text
        } else {
            ctx.style.palette.text_dim
        };
        let c = rect.center();
        let s = 4.5; // glyph half-extent
        match i {
            0 => ctx.paint.push(PaintCmd::Line {
                from: Pos2::new(c.x - s, c.y),
                to: Pos2::new(c.x + s, c.y),
                width: 1.2,
                color: col,
            }),
            1 => {
                let stroke_square = |rect: Rect| {
                    PaintCmd::Rrect(RectShape {
                        rect,
                        rounding: Rounding::same(1.5),
                        fill: None,
                        stroke: Some(Stroke {
                            width: 1.2,
                            color: col,
                        }),
                        shadow: None,
                        glow: None,
                    })
                };
                if maximized {
                    // Restore glyph: two offset squares.
                    let g = Rect::from_center_size(
                        Pos2::new(c.x - 1.0, c.y + 1.0),
                        Vec2::splat(2.0 * s - 2.0),
                    );
                    ctx.paint
                        .push(stroke_square(g.translate(Vec2::new(2.5, -2.5))));
                    ctx.paint.push(stroke_square(g));
                } else {
                    ctx.paint
                        .push(stroke_square(Rect::from_center_size(c, Vec2::splat(2.0 * s))));
                }
            }
            _ => {
                ctx.paint.push(PaintCmd::Line {
                    from: Pos2::new(c.x - s, c.y - s),
                    to: Pos2::new(c.x + s, c.y + s),
                    width: 1.2,
                    color: col,
                });
                ctx.paint.push(PaintCmd::Line {
                    from: Pos2::new(c.x + s, c.y - s),
                    to: Pos2::new(c.x - s, c.y + s),
                    width: 1.2,
                    color: col,
                });
            }
        }
        if ctx.input.pointer_pressed && hovered {
            match i {
                0 => act.minimize = true,
                1 => act.toggle_maximize = true,
                _ => act.close = true,
            }
        }
    }

    // Press on empty caption space (past the tabs, before the buttons)
    // starts an OS window move.
    if ctx.input.pointer_pressed {
        if let Some(p) = ctx.input.pointer_pos {
            if bar.contains(p) && p.x >= tabs_end_x && p.x < buttons_x {
                act.drag = true;
            }
        }
    }
    act
}

/// Window title + default client size for a float window hosting `tab`.
pub fn float_window_attrs(tab: &str) -> (String, u32, u32) {
    match dock_crusty::parse_tab(tab) {
        Some(EditorTab::Hierarchy) => ("Hierarchy".into(), 350, 600),
        Some(EditorTab::Inspector) => ("Inspector".into(), 400, 600),
        Some(EditorTab::AssetBrowser) => ("Assets".into(), 800, 500),
        Some(EditorTab::Console) => ("Console".into(), 700, 400),
        Some(EditorTab::Profiler) => ("Profiler".into(), 700, 500),
        Some(tab) => (tab.title_string(), 600, 500),
        None => (tab.to_string(), 600, 500),
    }
}
