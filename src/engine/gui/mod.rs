//! Custom egui-vulkano integration for Vulkano 0.34 and egui 0.33
//!
//! This module provides a minimal egui integration tailored for our engine.
//! Supports egui 0.33 with winit 0.30 event handling.

mod renderer;

pub use renderer::EguiRenderer;

use egui::Context;
use std::sync::Arc;
use winit::event::WindowEvent;
use winit::keyboard::{Key, NamedKey, PhysicalKey, KeyCode};
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

/// Main GUI integration struct
pub struct Gui {
    /// egui context (persistent across frames)
    egui_ctx: Context,
    /// Vulkan renderer for egui
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
    current_cursor: egui::CursorIcon,
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
            current_cursor: egui::CursorIcon::Default,
        })
    }

    /// Run GUI and render - call this once per frame
    pub fn render(
        &mut self,
        _window: &Window,
        swapchain_image: Arc<vulkano::image::Image>,
        mut ui_fn: impl FnMut(&egui::Context),
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
        let pointer_over_popup = self.egui_ctx.input(|i| i.pointer.hover_pos()).map_or(false, |pos| {
            self.egui_ctx.layer_id_at(pos).map_or(false, |layer| {
                // Block if layer order is above Background (popups, tooltips, etc.)
                layer.order > egui::Order::Background
            })
        });

        // Also check if egui is actively dragging AND the pointer started on a popup layer.
        // This handles the case where user starts dragging a slider in a popup and moves
        // the mouse outside the popup - we still want to block viewport input.
        let dragging_from_popup = self.egui_ctx.is_using_pointer() &&
            self.egui_ctx.input(|i| i.pointer.press_origin()).map_or(false, |origin_pos| {
                self.egui_ctx.layer_id_at(origin_pos).map_or(false, |layer| {
                    layer.order > egui::Order::Background
                })
            });

        let is_using_pointer = pointer_over_popup || dragging_from_popup;

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

        let command_buffer = self.renderer.render(
            swapchain_image,
            clipped_primitives,
            full_output.textures_delta,
            screen_rect,
        )?;

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
                    winit::event::Ime::Preedit(text, cursor) => {
                        if text.is_empty() {
                            // Preedit cleared
                            self.events.push(egui::Event::Ime(egui::ImeEvent::Preedit(String::new())));
                        } else {
                            // Preedit text with optional cursor position
                            let cursor_range = cursor.map(|(start, end)| {
                                egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(start),
                                    egui::text::CCursor::new(end),
                                )
                            });
                            self.events.push(egui::Event::Ime(egui::ImeEvent::Preedit(text.clone())));
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

            WindowEvent::KeyboardInput { event: key_event, .. } => {
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

            _ => false,
        }
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

    /// Register an external Vulkan image view as an egui texture
    ///
    /// Used for render-to-texture scenarios like the viewport.
    pub fn register_native_texture(
        &mut self,
        image_view: std::sync::Arc<vulkano::image::view::ImageView>,
    ) -> egui::TextureId {
        self.renderer.register_native_texture(image_view)
    }

    /// Update an existing native texture with a new image view
    ///
    /// Used when the viewport is resized and the texture needs to be recreated.
    pub fn update_native_texture(
        &mut self,
        texture_id: egui::TextureId,
        image_view: std::sync::Arc<vulkano::image::view::ImageView>,
    ) {
        self.renderer.update_native_texture(texture_id, image_view);
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
