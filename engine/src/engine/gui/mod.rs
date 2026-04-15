//! Custom egui-vulkano integration for Vulkano 0.34 and egui 0.33
//!
//! This module provides a minimal egui integration tailored for our engine.
//! Supports egui 0.33 with winit 0.30 event handling.

mod renderer;

pub use renderer::EguiRenderer;

use egui::Context;
use std::sync::Arc;
use winit::event::WindowEvent;
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::Window;

/// Result from GUI rendering including input consumption flags
pub struct GuiRenderResult {
    pub command_buffer: Arc<vulkano::command_buffer::PrimaryAutoCommandBuffer>,
    pub wants_keyboard: bool,
    pub wants_pointer: bool,
    /// True if egui is actively using the pointer (dragging a widget, not just hovering)
    pub is_using_pointer: bool,
    pub cursor_icon: egui::CursorIcon,
}

/// Result from GUI layout pass (no GPU commands).
pub struct GuiLayoutResult {
    pub clipped_primitives: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub screen_rect: egui::Rect,
    pub wants_keyboard: bool,
    pub wants_pointer: bool,
    pub is_using_pointer: bool,
    pub cursor_icon: egui::CursorIcon,
    pub texture_binds: Vec<crate::engine::rendering::frame_packet::TextureBindCommand>,
}

/// Main GUI integration struct
pub struct Gui {
    /// egui context (persistent across frames)
    egui_ctx: Context,
    /// Vulkan renderer for egui (kept for secondary windows that still render synchronously)
    renderer: EguiRenderer,
    /// Screen size for calculating input coordinates
    screen_size: [f32; 2],

    pointer_pos: Option<egui::Pos2>,
    modifiers: egui::Modifiers,
    events: Vec<egui::Event>,
    pixels_per_point: f32,
    /// Clipboard for copy/paste support
    clipboard: Option<arboard::Clipboard>,
    /// Current cursor icon
    _current_cursor: egui::CursorIcon,
    /// Viewport rect captured when pointer drag started.
    /// Used for consistent "outside viewport" detection during drags.
    drag_start_viewport_rect: Option<egui::Rect>,
    /// Files dropped onto the window from the OS (e.g. Windows Explorer).
    /// Drained each frame by the application.
    dropped_files: Vec<std::path::PathBuf>,
    /// True while the OS is hovering files over the window (before drop).
    hovered_file_count: usize,
    /// Queued texture bind commands for the render thread (egui texture slot protocol).
    pending_texture_binds: Vec<crate::engine::rendering::frame_packet::TextureBindCommand>,
    /// Next user texture ID counter for layout-only texture registration.
    next_texture_id: std::sync::atomic::AtomicU64,
}

impl Gui {
    /// Create new GUI integration
    pub fn new(
        device: Arc<vulkano::device::Device>,
        queue: Arc<vulkano::device::Queue>,
        swapchain_format: vulkano::format::Format,
        window: &Window,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let egui_ctx = Context::default();

        // Configure dark theme with better text visibility
        let mut visuals = egui::Visuals::dark();
        visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::from_gray(220);
        visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_gray(200);
        visuals.widgets.hovered.fg_stroke.color = egui::Color32::WHITE;
        visuals.widgets.active.fg_stroke.color = egui::Color32::WHITE;
        visuals.override_text_color = Some(egui::Color32::from_gray(230));
        egui_ctx.set_visuals(visuals);

        // Create Vulkan renderer
        let renderer = EguiRenderer::new(device, queue, swapchain_format)?;

        let size = window.inner_size();
        let screen_size = [size.width as f32, size.height as f32];

        // Initialize clipboard
        let clipboard = arboard::Clipboard::new().ok();

        Ok(Self {
            egui_ctx,
            renderer,
            screen_size,
            pointer_pos: None,
            modifiers: egui::Modifiers::default(),
            events: Vec::new(),
            pixels_per_point: 1.15, // Slightly larger for better readability
            clipboard,
            _current_cursor: egui::CursorIcon::Default,
            drag_start_viewport_rect: None,
            dropped_files: Vec::new(),
            hovered_file_count: 0,
            pending_texture_binds: Vec::new(),
            next_texture_id: std::sync::atomic::AtomicU64::new(1000),
        })
    }

    /// Layout-only pass: runs egui, tessellates, and returns primitives + deltas.
    /// No GPU commands are recorded. Call this when rendering is handled by the render thread.
    pub fn layout(
        &mut self,
        viewport_rect: Option<egui::Rect>,
        ui_fn: impl FnMut(&egui::Context),
    ) -> GuiLayoutResult {
        let (clipped_primitives, textures_delta, screen_rect, wants_keyboard, wants_pointer, is_using_pointer, cursor_icon) =
            self.layout_inner(viewport_rect, ui_fn);

        let texture_binds = std::mem::take(&mut self.pending_texture_binds);

        GuiLayoutResult {
            clipped_primitives,
            textures_delta,
            screen_rect,
            wants_keyboard,
            wants_pointer,
            is_using_pointer,
            cursor_icon,
            texture_binds,
        }
    }

    fn layout_inner(
        &mut self,
        viewport_rect: Option<egui::Rect>,
        mut ui_fn: impl FnMut(&egui::Context),
    ) -> (Vec<egui::ClippedPrimitive>, egui::TexturesDelta, egui::Rect, bool, bool, bool, egui::CursorIcon) {
        crate::profile_function!();

        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.screen_size[0], self.screen_size[1]),
            )),
            time: Some(current_time),
            predicted_dt: 1.0 / 60.0,
            modifiers: self.modifiers,
            events: std::mem::take(&mut self.events),
            ..Default::default()
        };

        let full_output = {
            crate::profile_scope!("egui_run");
            self.egui_ctx.run(raw_input, &mut ui_fn)
        };

        let wants_keyboard =
            full_output.platform_output.ime.is_some() || self.egui_ctx.wants_keyboard_input();
        let wants_pointer = self.egui_ctx.wants_pointer_input();

        let pointer_over_popup =
            self.egui_ctx
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|pos| {
                    self.egui_ctx.layer_id_at(pos).is_some_and(|layer| {
                        layer.order > egui::Order::Background
                    })
                });

        let dragging_from_popup = self.egui_ctx.is_using_pointer()
            && self
                .egui_ctx
                .input(|i| i.pointer.press_origin())
                .is_some_and(|origin_pos| {
                    self.egui_ctx
                        .layer_id_at(origin_pos)
                        .is_some_and(|layer| layer.order > egui::Order::Background)
                });

        let is_dragging = self.egui_ctx.is_using_pointer();
        let press_origin = self.egui_ctx.input(|i| i.pointer.press_origin());

        let effective_viewport_rect = if is_dragging {
            if self.drag_start_viewport_rect.is_none() {
                self.drag_start_viewport_rect = viewport_rect.map(|r| r.shrink(3.0));
            }
            self.drag_start_viewport_rect
        } else {
            self.drag_start_viewport_rect = None;
            None
        };

        let dragging_from_outside_viewport = is_dragging
            && effective_viewport_rect.is_some_and(|vp_rect| {
                press_origin.is_some_and(|origin_pos| !vp_rect.contains(origin_pos))
            });

        let is_using_pointer =
            pointer_over_popup || dragging_from_popup || dragging_from_outside_viewport;

        for command in &full_output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                if let Some(clipboard) = &mut self.clipboard {
                    let _ = clipboard.set_text(text);
                }
            }
        }

        let cursor_icon = full_output.platform_output.cursor_icon;
        self.pixels_per_point = full_output.pixels_per_point;

        let clipped_primitives = {
            crate::profile_scope!("egui_tessellate");
            self.egui_ctx
                .tessellate(full_output.shapes, full_output.pixels_per_point)
        };

        let screen_rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(self.screen_size[0], self.screen_size[1]),
        );

        (clipped_primitives, full_output.textures_delta, screen_rect, wants_keyboard, wants_pointer, is_using_pointer, cursor_icon)
    }

    /// Run GUI and render - call this once per frame
    ///
    /// `viewport_rect` is the previous frame's viewport rect (in egui screen coordinates),
    /// used to detect drags from outside the viewport (e.g., dock separator drags).
    pub fn render(
        &mut self,
        _window: &Window,
        swapchain_image: Arc<vulkano::image::Image>,
        viewport_rect: Option<egui::Rect>,
        ui_fn: impl FnMut(&egui::Context),
    ) -> Result<GuiRenderResult, Box<dyn std::error::Error>> {
        self.render_inner(_window, swapchain_image, viewport_rect, ui_fn, None)
    }

    /// Run GUI and render with the swapchain image cleared first.
    /// Use for secondary windows that have no prior 3D content.
    pub fn render_with_clear(
        &mut self,
        _window: &Window,
        swapchain_image: Arc<vulkano::image::Image>,
        viewport_rect: Option<egui::Rect>,
        ui_fn: impl FnMut(&egui::Context),
        clear_color: [f32; 4],
    ) -> Result<GuiRenderResult, Box<dyn std::error::Error>> {
        self.render_inner(_window, swapchain_image, viewport_rect, ui_fn, Some(clear_color))
    }

    fn render_inner(
        &mut self,
        _window: &Window,
        swapchain_image: Arc<vulkano::image::Image>,
        viewport_rect: Option<egui::Rect>,
        mut ui_fn: impl FnMut(&egui::Context),
        clear_color: Option<[f32; 4]>,
    ) -> Result<GuiRenderResult, Box<dyn std::error::Error>> {
        crate::profile_function!();

        // Build RawInput with collected events
        // Use unwrap_or to handle potential clock skew gracefully
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.screen_size[0], self.screen_size[1]),
            )),
            time: Some(current_time),
            predicted_dt: 1.0 / 60.0,
            modifiers: self.modifiers,
            events: std::mem::take(&mut self.events), // Take events, clearing the vec
            ..Default::default()
        };

        // Run egui with UI code
        let full_output = {
            crate::profile_scope!("egui_run");
            self.egui_ctx.run(raw_input, &mut ui_fn)
        };

        // Store what egui wants to consume
        let wants_keyboard =
            full_output.platform_output.ime.is_some() || self.egui_ctx.wants_keyboard_input();
        let wants_pointer = self.egui_ctx.wants_pointer_input();

        // Determine if egui is using the pointer for something OTHER than the viewport.
        // This is tricky because the viewport image itself has click_and_drag sense,
        // so is_using_pointer() returns true when clicking on viewport too!
        //
        // Solution: Check if pointer is over a popup/tooltip layer (above Background order).
        // Popups render on Foreground or higher layers. The viewport is on Background.
        // This way, we only block viewport input when interacting with popups/menus.
        let pointer_over_popup =
            self.egui_ctx
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|pos| {
                    self.egui_ctx.layer_id_at(pos).is_some_and(|layer| {
                        // Block if layer order is above Background (popups, tooltips, etc.)
                        layer.order > egui::Order::Background
                    })
                });

        // Also check if egui is actively dragging AND the pointer started on a popup layer.
        // This handles the case where user starts dragging a slider in a popup and moves
        // the mouse outside the popup - we still want to block viewport input.
        let dragging_from_popup = self.egui_ctx.is_using_pointer()
            && self
                .egui_ctx
                .input(|i| i.pointer.press_origin())
                .is_some_and(|origin_pos| {
                    self.egui_ctx
                        .layer_id_at(origin_pos)
                        .is_some_and(|layer| layer.order > egui::Order::Background)
                });

        // Block viewport input when dragging from OUTSIDE the viewport rect.
        // This catches egui_dock separator drags (which are on Background layer, not popup).
        //
        // To handle fast drags where viewport size changes between frames, we capture
        // the viewport rect at drag start and use it for the entire drag duration.
        // The rect is shrunk by separator width + buffer (3px) so separator clicks
        // are always detected as "outside".
        let is_dragging = self.egui_ctx.is_using_pointer();
        let press_origin = self.egui_ctx.input(|i| i.pointer.press_origin());

        // Manage drag start viewport rect state
        let effective_viewport_rect = if is_dragging {
            if self.drag_start_viewport_rect.is_none() {
                // New drag starting - capture viewport rect shrunk by separator width + buffer
                self.drag_start_viewport_rect = viewport_rect.map(|r| r.shrink(3.0));
            }
            self.drag_start_viewport_rect
        } else {
            // Not dragging - clear for next drag
            self.drag_start_viewport_rect = None;
            None
        };

        let dragging_from_outside_viewport = is_dragging
            && effective_viewport_rect.is_some_and(|vp_rect| {
                press_origin.is_some_and(|origin_pos| !vp_rect.contains(origin_pos))
            });

        let is_using_pointer =
            pointer_over_popup || dragging_from_popup || dragging_from_outside_viewport;

        // Handle clipboard copy via commands (egui 0.33 API)
        for command in &full_output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                if let Some(clipboard) = &mut self.clipboard {
                    let _ = clipboard.set_text(text);
                }
            }
        }

        // Get cursor icon
        let cursor_icon = full_output.platform_output.cursor_icon;

        // Update pixels_per_point for next frame
        self.pixels_per_point = full_output.pixels_per_point;

        // Tessellate and render
        let clipped_primitives = {
            crate::profile_scope!("egui_tessellate");
            self.egui_ctx
                .tessellate(full_output.shapes, full_output.pixels_per_point)
        };

        // Create screen rect from our stored size
        let screen_rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(self.screen_size[0], self.screen_size[1]),
        );

        let command_buffer = if let Some(color) = clear_color {
            self.renderer.render_with_clear(
                swapchain_image,
                clipped_primitives,
                full_output.textures_delta,
                screen_rect,
                color,
            )?
        } else {
            self.renderer.render(
                swapchain_image,
                clipped_primitives,
                full_output.textures_delta,
                screen_rect,
            )?
        };

        Ok(GuiRenderResult {
            command_buffer,
            wants_keyboard,
            wants_pointer,
            is_using_pointer,
            cursor_icon,
        })
    }

    /// Process winit 0.30 event and convert to egui event
    pub fn handle_event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer_pos = Some(egui::pos2(
                    position.x as f32 / self.pixels_per_point,
                    position.y as f32 / self.pixels_per_point,
                ));
                self.events
                    .push(egui::Event::PointerMoved(self.pointer_pos.unwrap()));
                true
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(pos) = self.pointer_pos {
                    if let Some(egui_button) = match button {
                        winit::event::MouseButton::Left => Some(egui::PointerButton::Primary),
                        winit::event::MouseButton::Right => Some(egui::PointerButton::Secondary),
                        winit::event::MouseButton::Middle => Some(egui::PointerButton::Middle),
                        _ => None,
                    } {
                        self.events.push(egui::Event::PointerButton {
                            pos,
                            button: egui_button,
                            pressed: state.is_pressed(),
                            modifiers: self.modifiers,
                        });
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let delta_vec = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        egui::vec2(*x * 20.0, *y * 20.0)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        egui::vec2(pos.x as f32, pos.y as f32) / self.pixels_per_point
                    }
                };

                self.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: delta_vec,
                    modifiers: self.modifiers,
                });
                true
            }

            WindowEvent::Ime(ime) => {
                match ime {
                    winit::event::Ime::Preedit(text, _cursor) => {
                        if text.is_empty() {
                            // Preedit cleared
                            self.events
                                .push(egui::Event::Ime(egui::ImeEvent::Preedit(String::new())));
                        } else {
                            self.events
                                .push(egui::Event::Ime(egui::ImeEvent::Preedit(text.clone())));
                        }
                        true
                    }
                    winit::event::Ime::Commit(text) => {
                        self.events.push(egui::Event::Text(text.clone()));
                        true
                    }
                    winit::event::Ime::Enabled => {
                        self.events.push(egui::Event::Ime(egui::ImeEvent::Enabled));
                        true
                    }
                    winit::event::Ime::Disabled => {
                        self.events.push(egui::Event::Ime(egui::ImeEvent::Disabled));
                        true
                    }
                }
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.modifiers = egui::Modifiers {
                    alt: state.alt_key(),
                    ctrl: state.control_key(),
                    shift: state.shift_key(),
                    mac_cmd: false,
                    command: state.control_key(), // On Windows/Linux, Ctrl is the command key
                };
                true
            }

            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                let pressed = key_event.state.is_pressed();

                // Update modifiers based on physical key
                if let PhysicalKey::Code(key) = key_event.physical_key {
                    match key {
                        KeyCode::ShiftLeft | KeyCode::ShiftRight => {
                            self.modifiers.shift = pressed;
                        }
                        KeyCode::ControlLeft | KeyCode::ControlRight => {
                            self.modifiers.ctrl = pressed;
                            self.modifiers.command = pressed;
                        }
                        KeyCode::AltLeft | KeyCode::AltRight => {
                            self.modifiers.alt = pressed;
                        }
                        _ => {}
                    }
                }

                // Translate to egui key
                if let Some(key) = translate_key(&key_event.logical_key) {
                    self.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: key_event.repeat,
                        modifiers: self.modifiers,
                    });

                    // Handle clipboard shortcuts (Ctrl+C, Ctrl+X, Ctrl+V)
                    if pressed && self.modifiers.ctrl {
                        match key {
                            egui::Key::C => {
                                self.events.push(egui::Event::Copy);
                            }
                            egui::Key::X => {
                                self.events.push(egui::Event::Cut);
                            }
                            egui::Key::V => {
                                // Get text from clipboard and push as Paste event
                                if let Some(clipboard) = &mut self.clipboard {
                                    if let Ok(text) = clipboard.get_text() {
                                        self.events.push(egui::Event::Paste(text));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Handle text input (replaces ReceivedCharacter in winit 0.30)
                if pressed && !key_event.repeat {
                    if let Key::Character(ch) = &key_event.logical_key {
                        // Only add text if no modifier keys are pressed (except shift)
                        if !self.modifiers.ctrl && !self.modifiers.alt && !self.modifiers.command {
                            self.events.push(egui::Event::Text(ch.to_string()));
                        }
                    }
                    // Space is Key::Named, not Key::Character, so handle it separately
                    if let Key::Named(NamedKey::Space) = &key_event.logical_key {
                        if !self.modifiers.ctrl && !self.modifiers.alt && !self.modifiers.command {
                            self.events.push(egui::Event::Text(" ".to_string()));
                        }
                    }
                }

                true
            }

            WindowEvent::DroppedFile(path) => {
                self.dropped_files.push(path.clone());
                self.hovered_file_count = 0;
                true
            }

            WindowEvent::HoveredFile(_path) => {
                self.hovered_file_count += 1;
                true
            }

            WindowEvent::HoveredFileCancelled => {
                self.hovered_file_count = 0;
                true
            }

            _ => false,
        }
    }

    /// Drain files dropped from the OS this frame.
    pub fn take_dropped_files(&mut self) -> Vec<std::path::PathBuf> {
        std::mem::take(&mut self.dropped_files)
    }

    /// Returns true if the OS is currently hovering files over the window.
    pub fn is_hovering_external_files(&self) -> bool {
        self.hovered_file_count > 0
    }

    /// Update screen size (call when window is resized)
    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        // Skip zero-size updates (window minimized) to prevent NaN in egui layout
        if width > 0.0 && height > 0.0 {
            self.screen_size = [width, height];
        }
    }

    /// Get egui context for custom usage
    pub fn context(&self) -> &Context {
        &self.egui_ctx
    }

    /// Register an external Vulkan image view as an egui texture.
    /// Queues a TextureBindCommand for the render thread.
    pub fn register_native_texture(
        &mut self,
        image_view: std::sync::Arc<vulkano::image::view::ImageView>,
    ) -> egui::TextureId {
        let id = self.next_texture_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let texture_id = egui::TextureId::User(id);
        self.pending_texture_binds.push(
            crate::engine::rendering::frame_packet::TextureBindCommand {
                texture_id,
                image_view,
            },
        );
        texture_id
    }

    /// Update an existing native texture with a new image view.
    /// Queues a TextureBindCommand for the render thread.
    pub fn update_native_texture(
        &mut self,
        texture_id: egui::TextureId,
        image_view: std::sync::Arc<vulkano::image::view::ImageView>,
    ) {
        self.pending_texture_binds.push(
            crate::engine::rendering::frame_packet::TextureBindCommand {
                texture_id,
                image_view,
            },
        );
    }

    /// Clear framebuffer cache (call on swapchain recreation)
    pub fn clear_framebuffer_cache(&mut self) {
        self.renderer.clear_framebuffer_cache();
    }
}

/// Translate winit 0.30 Key to egui Key
fn translate_key(key: &Key) -> Option<egui::Key> {
    match key {
        Key::Named(named) => Some(match named {
            NamedKey::Escape => egui::Key::Escape,
            NamedKey::Enter => egui::Key::Enter,
            NamedKey::Tab => egui::Key::Tab,
            NamedKey::Space => egui::Key::Space,
            NamedKey::Backspace => egui::Key::Backspace,
            NamedKey::Delete => egui::Key::Delete,
            NamedKey::Insert => egui::Key::Insert,
            NamedKey::Home => egui::Key::Home,
            NamedKey::End => egui::Key::End,
            NamedKey::PageUp => egui::Key::PageUp,
            NamedKey::PageDown => egui::Key::PageDown,
            NamedKey::ArrowLeft => egui::Key::ArrowLeft,
            NamedKey::ArrowRight => egui::Key::ArrowRight,
            NamedKey::ArrowUp => egui::Key::ArrowUp,
            NamedKey::ArrowDown => egui::Key::ArrowDown,
            NamedKey::F1 => egui::Key::F1,
            NamedKey::F2 => egui::Key::F2,
            NamedKey::F3 => egui::Key::F3,
            NamedKey::F4 => egui::Key::F4,
            NamedKey::F5 => egui::Key::F5,
            NamedKey::F6 => egui::Key::F6,
            NamedKey::F7 => egui::Key::F7,
            NamedKey::F8 => egui::Key::F8,
            NamedKey::F9 => egui::Key::F9,
            NamedKey::F10 => egui::Key::F10,
            NamedKey::F11 => egui::Key::F11,
            NamedKey::F12 => egui::Key::F12,
            _ => return None,
        }),
        Key::Character(ch) => {
            // Get the first character
            let c = ch.chars().next()?;
            match c.to_ascii_uppercase() {
                'A' => Some(egui::Key::A),
                'B' => Some(egui::Key::B),
                'C' => Some(egui::Key::C),
                'D' => Some(egui::Key::D),
                'E' => Some(egui::Key::E),
                'F' => Some(egui::Key::F),
                'G' => Some(egui::Key::G),
                'H' => Some(egui::Key::H),
                'I' => Some(egui::Key::I),
                'J' => Some(egui::Key::J),
                'K' => Some(egui::Key::K),
                'L' => Some(egui::Key::L),
                'M' => Some(egui::Key::M),
                'N' => Some(egui::Key::N),
                'O' => Some(egui::Key::O),
                'P' => Some(egui::Key::P),
                'Q' => Some(egui::Key::Q),
                'R' => Some(egui::Key::R),
                'S' => Some(egui::Key::S),
                'T' => Some(egui::Key::T),
                'U' => Some(egui::Key::U),
                'V' => Some(egui::Key::V),
                'W' => Some(egui::Key::W),
                'X' => Some(egui::Key::X),
                'Y' => Some(egui::Key::Y),
                'Z' => Some(egui::Key::Z),
                '0' => Some(egui::Key::Num0),
                '1' => Some(egui::Key::Num1),
                '2' => Some(egui::Key::Num2),
                '3' => Some(egui::Key::Num3),
                '4' => Some(egui::Key::Num4),
                '5' => Some(egui::Key::Num5),
                '6' => Some(egui::Key::Num6),
                '7' => Some(egui::Key::Num7),
                '8' => Some(egui::Key::Num8),
                '9' => Some(egui::Key::Num9),
                _ => None,
            }
        }
        _ => None,
    }
}
