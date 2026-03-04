//! TabViewer implementation for egui_dock
//!
//! Handles rendering of each tab type in the dock layout.

use super::{
    asset_browser::AssetBrowserPanel,
    console::{ConsoleLog, LogFilter, LogLevel, LogMessage},
    console_cmd::{CommandContext, ConsoleCommandSystem},
    dock_layout::EditorTab,
    icons::IconManager,
    menu_bar::MenuAction,
    profiler::ProfilerPanel,
    viewport::{
        render_viewport_toolbar_overlay, CameraControlMode, EditorCamera, GizmoHandler,
        GizmoInteractionResult, ToolMode, ViewportSettings,
    },
    CommandHistory, HierarchyPanel, InspectorPanel, Selection,
};
use crate::engine::ecs::components::Transform;
use crate::engine::ecs::resources::PlayMode;
use egui::{Color32, RichText, Ui};
use egui_dock::TabViewer;
use hecs::World;

/// Context passed to tab viewer for rendering panels
pub struct EditorContext<'a> {
    pub world: &'a mut World,
    pub selection: &'a mut Selection,
    pub hierarchy_panel: &'a mut HierarchyPanel,
    pub inspector_panel: &'a mut InspectorPanel,
    pub command_history: &'a mut CommandHistory,
    /// Show profiler flag
    pub show_profiler: &'a mut bool,
    /// Console log messages (mutable for command output)
    pub console_messages: &'a mut ConsoleLog,
    /// Console log filter settings
    pub log_filter: &'a mut LogFilter,
    /// Viewport texture ID for rendering the 3D scene
    pub viewport_texture_id: Option<egui::TextureId>,
    /// Current viewport size (for detecting resize)
    pub viewport_size: &'a mut (u32, u32),
    /// Profiler panel
    pub profiler_panel: &'a mut ProfilerPanel,
    /// Console command system
    pub console_command_system: &'a mut ConsoleCommandSystem,
    /// Console input text
    pub console_input: &'a mut String,
    /// Toggle for stat fps overlay (Unreal-style)
    pub show_stat_fps: &'a mut bool,
    /// Current FPS for overlay display
    pub fps: f32,
    /// Current frame time in milliseconds for overlay display
    pub delta_ms: f32,
    /// Editor camera for viewport controls
    pub editor_camera: &'a mut EditorCamera,
    /// Gizmo handler for transform manipulation
    pub gizmo_handler: &'a mut GizmoHandler,
    /// Grid visibility toggle
    pub grid_visible: &'a mut bool,
    /// Output: viewport is hovered (set during render_viewport)
    pub viewport_hovered: &'a mut bool,
    /// Output: viewport rect in screen coordinates (set during render_viewport)
    pub viewport_rect: &'a mut egui::Rect,
    /// Viewport settings (tool mode, snapping, camera speed)
    pub viewport_settings: &'a mut ViewportSettings,
    /// Icon manager for toolbar icons (optional, falls back to text if None)
    pub icon_manager: Option<&'a IconManager>,
    /// Asset browser panel
    pub asset_browser: &'a mut AssetBrowserPanel,
    /// Current play mode for viewport indicator
    pub play_mode: PlayMode,
    /// Output: play-mode action from viewport tab bar (set during render)
    pub toolbar_action: &'a mut MenuAction,
}

/// Tab viewer that renders each panel type
pub struct EditorTabViewer<'a> {
    pub editor: EditorContext<'a>,
}

impl<'a> TabViewer for EditorTabViewer<'a> {
    type Tab = EditorTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            EditorTab::Viewport => "".into(),
            _ => tab.title().into(),
        }
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
        match tab {
            EditorTab::Viewport => self.render_viewport(ui),
            EditorTab::Hierarchy => self.render_hierarchy(ui),
            EditorTab::Inspector => self.render_inspector(ui),
            EditorTab::AssetBrowser => self.render_asset_browser(ui),
            EditorTab::Console => self.render_console(ui),
            EditorTab::Profiler => self.render_profiler(ui),
        }
    }

    fn closeable(&mut self, tab: &mut Self::Tab) -> bool {
        tab.closable()
    }
}

impl<'a> EditorTabViewer<'a> {
    /// Render play/pause/stop icons centered on the viewport's tab bar header.
    /// Uses a floating egui::Area positioned over the tab bar strip.
    fn render_play_controls_on_tab_bar(
        &mut self,
        ctx: &egui::Context,
        viewport_content_rect: egui::Rect,
    ) {
        use super::menu_bar::render_play_controls;

        let icon_size = 18.0_f32;
        let icon_spacing = 4.0_f32;
        let icon_count = 3.0_f32;
        let cluster_w = icon_count * icon_size + (icon_count - 1.0) * icon_spacing;
        let pill_pad_x = 6.0_f32;
        let pill_pad_y = 3.0_f32;
        let pill_w = cluster_w + pill_pad_x * 2.0;
        let pill_h = icon_size + pill_pad_y * 2.0;

        let tab_bar_height = 24.0;
        let tab_bar_top = viewport_content_rect.top() - tab_bar_height;
        let bar_center_x = viewport_content_rect.center().x;
        let pill_top = tab_bar_top + (tab_bar_height - pill_h) / 2.0;

        let pill_bg = Color32::from_rgba_premultiplied(50, 50, 50, 200);

        egui::Area::new(egui::Id::new("play_controls_tab_bar"))
            .fixed_pos(egui::pos2(bar_center_x - pill_w / 2.0, pill_top))
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                let (pill_rect, _) =
                    ui.allocate_exact_size(egui::vec2(pill_w, pill_h), egui::Sense::hover());

                ui.painter().rect_filled(pill_rect, pill_h / 2.0, pill_bg);

                let icons_rect = pill_rect.shrink2(egui::vec2(pill_pad_x, pill_pad_y));
                let mut child_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(icons_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                child_ui.spacing_mut().item_spacing.x = icon_spacing;
                render_play_controls(
                    &mut child_ui,
                    self.editor.play_mode,
                    self.editor.icon_manager,
                    self.editor.toolbar_action,
                );
            });
    }

    /// Render the 3D viewport with the rendered scene texture
    fn render_viewport(&mut self, ui: &mut Ui) {
        // Get available size for the viewport (full panel area now, no toolbar taking space)
        let available_size = ui.available_size();

        // Track the desired viewport size (for texture resizing)
        let new_width = (available_size.x.max(1.0)) as u32;
        let new_height = (available_size.y.max(1.0)) as u32;
        *self.editor.viewport_size = (new_width, new_height);

        // If we have a viewport texture, display it
        // Use the response rect for gizmo positioning (not available_rect_before_wrap)
        let mut viewport_rect = ui.available_rect_before_wrap();

        // Render play controls centered on the tab bar (above viewport content)
        self.render_play_controls_on_tab_bar(ui.ctx(), viewport_rect);

        // Render Unreal-style toolbar as floating overlay FIRST
        // This ensures it gets input priority over the viewport image below
        let camera_mode = self.editor.editor_camera.current_mode();
        render_viewport_toolbar_overlay(
            ui.ctx(),
            viewport_rect,
            self.editor.viewport_settings,
            camera_mode,
            self.editor.icon_manager,
        );

        if let Some(texture_id) = self.editor.viewport_texture_id {
            // Display the rendered scene texture filling the available space
            let response = ui.add(
                egui::Image::new(egui::load::SizedTexture::new(
                    texture_id,
                    egui::vec2(available_size.x, available_size.y),
                ))
                .sense(egui::Sense::click_and_drag()),
            );

            // Use the ACTUAL image rect for gizmo positioning
            viewport_rect = response.rect;

            // Store viewport rect for next frame's input blocking
            *self.editor.viewport_rect = viewport_rect;

            // Use contains_pointer() instead of hovered() to allow viewport interaction
            // even after clicking in other panels (like hierarchy). hovered() returns false
            // if any other widget is being interacted with, but contains_pointer() just
            // checks if the mouse is physically over the viewport.
            let viewport_hovered = response.contains_pointer();
            *self.editor.viewport_hovered = viewport_hovered;

            // Handle keyboard shortcuts when viewport is hovered
            // Block gizmo mode shortcuts during camera drag (but WASD still works for camera movement)
            let camera_dragging =
                self.editor.editor_camera.current_mode() != CameraControlMode::None;

            if viewport_hovered && !camera_dragging {
                let input = ui.input(|i| i.clone());

                // Tool mode shortcuts (Q, W, E, R)
                if input.key_pressed(egui::Key::Q) {
                    self.editor.viewport_settings.tool_mode = ToolMode::Select;
                }
                if input.key_pressed(egui::Key::W) {
                    self.editor.viewport_settings.tool_mode = ToolMode::Translate;
                }
                if input.key_pressed(egui::Key::E) {
                    self.editor.viewport_settings.tool_mode = ToolMode::Rotate;
                }
                if input.key_pressed(egui::Key::R) {
                    self.editor.viewport_settings.tool_mode = ToolMode::Scale;
                }
                if input.key_pressed(egui::Key::G) {
                    self.editor.viewport_settings.grid_visible =
                        !self.editor.viewport_settings.grid_visible;
                }

                // Snapping while Ctrl held - enable appropriate snap for current mode
                if input.modifiers.ctrl {
                    match self.editor.viewport_settings.tool_mode {
                        ToolMode::Select | ToolMode::Translate => {
                            self.editor.viewport_settings.grid_snap_enabled = true;
                        }
                        ToolMode::Rotate => {
                            self.editor.viewport_settings.rotation_snap_enabled = true;
                        }
                        ToolMode::Scale => {
                            self.editor.viewport_settings.scale_snap_enabled = true;
                        }
                    }
                }
            }

            // Sync viewport settings to gizmo handler
            self.editor.gizmo_handler.mode = self.editor.viewport_settings.tool_mode;
            self.editor.gizmo_handler.orientation = self.editor.viewport_settings.gizmo_orientation;
            self.editor.gizmo_handler.snap_translate = self.editor.viewport_settings.snap_translate;
            self.editor.gizmo_handler.snap_rotate = self.editor.viewport_settings.snap_rotate;
            self.editor.gizmo_handler.snap_scale = self.editor.viewport_settings.snap_scale;
            self.editor.gizmo_handler.snapping_enabled =
                self.editor.viewport_settings.current_snap_enabled();
            *self.editor.grid_visible = self.editor.viewport_settings.grid_visible;

            // Process gizmo only in Edit mode
            let gizmo_allowed = self.editor.play_mode == PlayMode::Edit;
            if gizmo_allowed && self.editor.gizmo_handler.should_show_gizmo() {
                if let Some(entity) = self.editor.selection.primary() {
                    let view = self.editor.editor_camera.view_matrix();
                    // Use gizmo projection (no Vulkan Y-flip) for egui rendering
                    let proj = self.editor.editor_camera.projection_matrix_for_gizmo();

                    let gizmo_result = self.editor.gizmo_handler.update(
                        ui,
                        view,
                        proj,
                        viewport_rect,
                        Some(entity),
                        self.editor.world,
                    );

                    match gizmo_result {
                        GizmoInteractionResult::Transforming {
                            entity,
                            new_transform,
                        } => {
                            // Apply transform immediately during drag
                            if let Ok(mut t) = self.editor.world.get::<&mut Transform>(entity) {
                                *t = new_transform;
                            }
                            crate::engine::ecs::hierarchy::mark_transform_dirty(
                                self.editor.world,
                                entity,
                            );
                        }
                        GizmoInteractionResult::DragEnded {
                            entity,
                            start_transform,
                            end_transform,
                        } => {
                            // Create undo command
                            use super::TransformChangeCommand;
                            let cmd = TransformChangeCommand::new(
                                entity,
                                &start_transform,
                                &end_transform,
                            );
                            self.editor
                                .command_history
                                .execute(Box::new(cmd), self.editor.world);
                        }
                        GizmoInteractionResult::None => {}
                    }
                }
            }
        } else {
            // Fallback placeholder when texture isn't available
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(available_size.y / 2.0 - 40.0);
                    ui.heading("3D Viewport");
                    ui.label(format!(
                        "Size: {:.0} x {:.0}",
                        available_size.x, available_size.y
                    ));
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Initializing viewport texture...").weak());
                });
            });
        }

        // Render play mode indicator bar at top of viewport
        match self.editor.play_mode {
            PlayMode::Playing => {
                let painter = ui.painter();
                let bar_rect = egui::Rect::from_min_size(
                    viewport_rect.left_top(),
                    egui::vec2(viewport_rect.width(), 3.0),
                );
                painter.rect_filled(bar_rect, 0.0, Color32::from_rgb(50, 200, 50));
            }
            PlayMode::Paused => {
                let painter = ui.painter();
                let bar_rect = egui::Rect::from_min_size(
                    viewport_rect.left_top(),
                    egui::vec2(viewport_rect.width(), 3.0),
                );
                painter.rect_filled(bar_rect, 0.0, Color32::from_rgb(220, 200, 50));
            }
            PlayMode::Edit => {}
        }

        // Render stat fps overlay if enabled (Unreal Engine style)
        // Position below the toolbar overlay
        if *self.editor.show_stat_fps {
            let painter = ui.painter();
            let padding = 8.0;
            let line_height = 18.0;

            // Position in top-left corner of viewport (below toolbar overlay ~36px)
            let overlay_pos = egui::pos2(
                viewport_rect.left() + padding,
                viewport_rect.top() + padding + 36.0,
            );

            // Format stats
            let fps_text = format!("FPS: {:.1}", self.editor.fps);
            let ms_text = format!("Frame: {:.2} ms", self.editor.delta_ms);

            // Calculate background size
            let text_width = 120.0;
            let bg_rect = egui::Rect::from_min_size(
                overlay_pos,
                egui::vec2(text_width, line_height * 2.0 + padding),
            );

            // Draw semi-transparent background
            painter.rect_filled(
                bg_rect,
                4.0, // corner radius
                Color32::from_rgba_unmultiplied(0, 0, 0, 180),
            );

            // Draw FPS text
            painter.text(
                egui::pos2(overlay_pos.x + 4.0, overlay_pos.y + 2.0),
                egui::Align2::LEFT_TOP,
                &fps_text,
                egui::FontId::monospace(14.0),
                Color32::from_rgb(100, 255, 100), // Green for FPS
            );

            // Draw frame time text
            painter.text(
                egui::pos2(overlay_pos.x + 4.0, overlay_pos.y + line_height + 2.0),
                egui::Align2::LEFT_TOP,
                &ms_text,
                egui::FontId::monospace(14.0),
                Color32::from_rgb(255, 255, 100), // Yellow for frame time
            );
        }
    }

    /// Render the hierarchy panel
    fn render_hierarchy(&mut self, ui: &mut Ui) {
        self.editor.hierarchy_panel.show_contents(
            ui,
            self.editor.world,
            self.editor.selection,
            self.editor.play_mode,
        );
    }

    /// Render the inspector panel
    fn render_inspector(&mut self, ui: &mut Ui) {
        self.editor.inspector_panel.show_contents(
            ui,
            self.editor.world,
            self.editor.selection,
            self.editor.play_mode,
        );
    }

    /// Render the asset browser panel
    fn render_asset_browser(&mut self, ui: &mut Ui) {
        self.editor.asset_browser.show(ui, self.editor.icon_manager);
    }

    /// Render the console panel
    fn render_console(&mut self, ui: &mut Ui) {
        let (info_count, warn_count, error_count) = self.editor.console_messages.counts();

        // Header with filter toggles (custom styled buttons instead of selectable_label)
        ui.horizontal(|ui| {
            ui.heading("Console");
            ui.separator();

            // Error filter button - dark red when active
            let error_fill = if self.editor.log_filter.show_error {
                Color32::from_rgba_unmultiplied(100, 50, 50, 180)
            } else {
                Color32::from_gray(45)
            };
            let error_text = RichText::new(format!("Errors ({})", error_count)).color(
                if self.editor.log_filter.show_error {
                    LogLevel::Error.color()
                } else {
                    Color32::GRAY
                },
            );
            if ui
                .add(
                    egui::Button::new(error_text)
                        .fill(error_fill)
                        .corner_radius(3.0),
                )
                .clicked()
            {
                self.editor.log_filter.show_error = !self.editor.log_filter.show_error;
            }

            // Warning filter button - dark orange when active
            let warn_fill = if self.editor.log_filter.show_warning {
                Color32::from_rgba_unmultiplied(100, 80, 40, 180)
            } else {
                Color32::from_gray(45)
            };
            let warn_text = RichText::new(format!("Warnings ({})", warn_count)).color(
                if self.editor.log_filter.show_warning {
                    LogLevel::Warning.color()
                } else {
                    Color32::GRAY
                },
            );
            if ui
                .add(
                    egui::Button::new(warn_text)
                        .fill(warn_fill)
                        .corner_radius(3.0),
                )
                .clicked()
            {
                self.editor.log_filter.show_warning = !self.editor.log_filter.show_warning;
            }

            // Info filter button - dark blue-gray when active
            let info_fill = if self.editor.log_filter.show_info {
                Color32::from_rgba_unmultiplied(60, 70, 90, 180)
            } else {
                Color32::from_gray(45)
            };
            let info_text = RichText::new(format!("Info ({})", info_count)).color(
                if self.editor.log_filter.show_info {
                    LogLevel::Info.color()
                } else {
                    Color32::GRAY
                },
            );
            if ui
                .add(
                    egui::Button::new(info_text)
                        .fill(info_fill)
                        .corner_radius(3.0),
                )
                .clicked()
            {
                self.editor.log_filter.show_info = !self.editor.log_filter.show_info;
            }
        });
        ui.separator();

        // Message display area
        let available_height = ui.available_height() - 30.0; // Reserve space for input
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height(available_height.max(50.0))
            .show(ui, |ui| {
                // Enable text selection for console messages
                ui.style_mut().interaction.selectable_labels = true;

                let mut shown_count = 0;
                for message in self.editor.console_messages.iter() {
                    if self.editor.log_filter.should_show(message) {
                        ui.label(message.rich_text());
                        shown_count += 1;
                    }
                }

                if shown_count == 0 {
                    ui.label(RichText::new("No messages").weak().italics());
                }
            });

        ui.separator();

        // Command input field
        let response = ui.add(
            egui::TextEdit::singleline(self.editor.console_input)
                .hint_text("Enter command (type 'help' for available commands)")
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace),
        );

        // Handle Enter key to execute command
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let input = std::mem::take(self.editor.console_input);
            if !input.is_empty() {
                // Echo the command input
                self.editor
                    .console_messages
                    .push(LogMessage::info(format!("> {}", input)));

                // Create execution context with access to toggles
                let mut ctx = CommandContext::new(self.editor.world, self.editor.show_stat_fps);

                // Execute command
                let output = self.editor.console_command_system.execute(&input, &mut ctx);

                // Handle clear command specially
                if output.len() == 1 && output[0].text == "__CLEAR__" {
                    self.editor.console_messages.clear();
                } else {
                    // Append output messages
                    self.editor.console_messages.extend(output);
                }
            }
            // Re-focus the input field
            response.request_focus();
        }

        // Handle keyboard shortcuts when input is focused
        if response.has_focus() {
            // Up/Down for history navigation
            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                if let Some(prev) = self
                    .editor
                    .console_command_system
                    .history
                    .previous(self.editor.console_input)
                {
                    *self.editor.console_input = prev.to_string();
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                if let Some(next) = self.editor.console_command_system.history.navigate_next() {
                    *self.editor.console_input = next.to_string();
                }
            }
            // Escape to clear input
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.editor.console_input.clear();
            }
        }
    }

    /// Render the profiler panel
    fn render_profiler(&mut self, ui: &mut Ui) {
        self.editor.profiler_panel.show_contents(ui);
    }
}
