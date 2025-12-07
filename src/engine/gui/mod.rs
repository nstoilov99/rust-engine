//! Custom egui-vulkano integration for Vulkano 0.34
//!
//! This module provides a minimal egui integration tailored for our engine.
//! Based on patterns from egui_winit_vulkano but adapted for Vulkano 0.34.

mod renderer;

pub use renderer::EguiRenderer;

use egui::Context;
use std::sync::Arc;
use winit::event::VirtualKeyCode;
use winit::event::WindowEvent;
use winit::window::Window;

/// Result from GUI rendering including input consumption flags
pub struct GuiRenderResult {
    pub command_buffer: Arc<
        vulkano::command_buffer::PrimaryAutoCommandBuffer<
            Arc<vulkano::command_buffer::allocator::StandardCommandBufferAllocator>,
        >,
    >,
    pub wants_keyboard: bool,
    pub wants_pointer: bool,
}

/// Main GUI integration struct
pub struct Gui {
    /// egui context
    context: Context,
    /// Vulkan renderer for egui
    renderer: EguiRenderer,
    /// Screen size for calculating input coordinates
    screen_size: [f32; 2],

    pointer_pos: Option<egui::Pos2>,
    modifiers: egui::Modifiers,
    events: Vec<egui::Event>,
    pixels_per_point: f32,
}

impl Gui {
    /// Create new GUI integration
    pub fn new(
        device: Arc<vulkano::device::Device>,
        queue: Arc<vulkano::device::Queue>,
        swapchain_format: vulkano::format::Format,
        window: &Window,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let context = Context::default();

        // Create Vulkan renderer
        let renderer = EguiRenderer::new(device, queue, swapchain_format)?;

        let size = window.inner_size();
        let screen_size = [size.width as f32, size.height as f32];

        Ok(Self {
            context,
            renderer,
            screen_size,
            pointer_pos: None,
            modifiers: egui::Modifiers::default(),
            events: Vec::new(),
            pixels_per_point: 1.0,
        })
    }

    /// Run GUI and render - call this once per frame
    pub fn render(
        &mut self,
        _window: &Window,
        swapchain_image: Arc<vulkano::image::Image>,
        mut ui_fn: impl FnMut(&egui::Context),
    ) -> Result<GuiRenderResult, Box<dyn std::error::Error>> {
        // Build RawInput with collected events
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.screen_size[0], self.screen_size[1]),
            )),
            time: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64(),
            ),
            predicted_dt: 1.0 / 60.0,
            modifiers: self.modifiers,
            events: std::mem::take(&mut self.events), // Take events, clearing the vec
            ..Default::default()
        };

        // Run egui with UI code
        let full_output = self.context.run(raw_input, &mut ui_fn);

        // Store what egui wants to consume
        let wants_keyboard =
            full_output.platform_output.ime.is_some() || self.context.wants_keyboard_input();
        let wants_pointer = self.context.wants_pointer_input();

        // Update pixels_per_point for next frame
        self.pixels_per_point = full_output.pixels_per_point;

        // Tessellate and render
        let clipped_primitives = self
            .context
            .tessellate(full_output.shapes, full_output.pixels_per_point);

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
        })
    }

    /// Process winit event and convert to egui event
    pub fn handle_event(&mut self, event: &winit::event::WindowEvent) -> bool {
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
                            pressed: *state == winit::event::ElementState::Pressed,
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

            WindowEvent::ReceivedCharacter(ch) => {
                if !ch.is_control() {
                    self.events.push(egui::Event::Text(ch.to_string()));
                    true
                } else {
                    false
                }
            }

            WindowEvent::KeyboardInput { input, .. } => {
                if let Some(keycode) = input.virtual_keycode {
                    let pressed = input.state == winit::event::ElementState::Pressed;

                    // Update modifiers
                    match keycode {
                        VirtualKeyCode::LShift | VirtualKeyCode::RShift => {
                            self.modifiers.shift = pressed;
                        }
                        VirtualKeyCode::LControl | VirtualKeyCode::RControl => {
                            self.modifiers.ctrl = pressed;
                        }
                        VirtualKeyCode::LAlt | VirtualKeyCode::RAlt => {
                            self.modifiers.alt = pressed;
                        }
                        VirtualKeyCode::LWin | VirtualKeyCode::RWin => {
                            self.modifiers.mac_cmd = pressed;
                            self.modifiers.command = pressed;
                        }
                        _ => {}
                    }

                    // Convert to egui key
                    if let Some(key) = translate_virtual_key_code(keycode) {
                        self.events.push(egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed,
                            repeat: false,
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

            _ => false,
        }
    }

    /// Update screen size (call when window is resized)
    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        self.screen_size = [width, height];
    }

    /// Get egui context for custom usage
    pub fn context(&self) -> &Context {
        &self.context
    }
}

fn translate_virtual_key_code(key: winit::event::VirtualKeyCode) -> Option<egui::Key> {
    use winit::event::VirtualKeyCode;

    Some(match key {
        VirtualKeyCode::Escape => egui::Key::Escape,
        VirtualKeyCode::Insert => egui::Key::Insert,
        VirtualKeyCode::Home => egui::Key::Home,
        VirtualKeyCode::Delete => egui::Key::Delete,
        VirtualKeyCode::End => egui::Key::End,
        VirtualKeyCode::PageDown => egui::Key::PageDown,
        VirtualKeyCode::PageUp => egui::Key::PageUp,
        VirtualKeyCode::Left => egui::Key::ArrowLeft,
        VirtualKeyCode::Up => egui::Key::ArrowUp,
        VirtualKeyCode::Right => egui::Key::ArrowRight,
        VirtualKeyCode::Down => egui::Key::ArrowDown,
        VirtualKeyCode::Back => egui::Key::Backspace,
        VirtualKeyCode::Return => egui::Key::Enter,
        VirtualKeyCode::Tab => egui::Key::Tab,
        VirtualKeyCode::Space => egui::Key::Space,

        VirtualKeyCode::A => egui::Key::A,
        VirtualKeyCode::B => egui::Key::B,
        VirtualKeyCode::C => egui::Key::C,
        VirtualKeyCode::D => egui::Key::D,
        VirtualKeyCode::E => egui::Key::E,
        VirtualKeyCode::F => egui::Key::F,
        VirtualKeyCode::G => egui::Key::G,
        VirtualKeyCode::H => egui::Key::H,
        VirtualKeyCode::I => egui::Key::I,
        VirtualKeyCode::J => egui::Key::J,
        VirtualKeyCode::K => egui::Key::K,
        VirtualKeyCode::L => egui::Key::L,
        VirtualKeyCode::M => egui::Key::M,
        VirtualKeyCode::N => egui::Key::N,
        VirtualKeyCode::O => egui::Key::O,
        VirtualKeyCode::P => egui::Key::P,
        VirtualKeyCode::Q => egui::Key::Q,
        VirtualKeyCode::R => egui::Key::R,
        VirtualKeyCode::S => egui::Key::S,
        VirtualKeyCode::T => egui::Key::T,
        VirtualKeyCode::U => egui::Key::U,
        VirtualKeyCode::V => egui::Key::V,
        VirtualKeyCode::W => egui::Key::W,
        VirtualKeyCode::X => egui::Key::X,
        VirtualKeyCode::Y => egui::Key::Y,
        VirtualKeyCode::Z => egui::Key::Z,

        VirtualKeyCode::Key0 => egui::Key::Num0,
        VirtualKeyCode::Key1 => egui::Key::Num1,
        VirtualKeyCode::Key2 => egui::Key::Num2,
        VirtualKeyCode::Key3 => egui::Key::Num3,
        VirtualKeyCode::Key4 => egui::Key::Num4,
        VirtualKeyCode::Key5 => egui::Key::Num5,
        VirtualKeyCode::Key6 => egui::Key::Num6,
        VirtualKeyCode::Key7 => egui::Key::Num7,
        VirtualKeyCode::Key8 => egui::Key::Num8,
        VirtualKeyCode::Key9 => egui::Key::Num9,

        _ => return None,
    })
}
