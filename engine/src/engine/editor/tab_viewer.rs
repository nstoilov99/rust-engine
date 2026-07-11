//! TabViewer implementation for egui_dock
//!
//! Handles rendering of each tab type in the dock layout.

use super::{
    asset_browser::AssetBrowserPanel,
    console::{ConsoleLog, LogFilter},
    console_cmd::ConsoleCommandSystem,
    dock_layout::EditorTab,
    icons::IconManager,
    input_action_editor::{InputActionEditor, InputActionEditorState},
    input_context_editor::{InputContextEditor, InputContextEditorState},
    menu_bar::MenuAction,
    profiler::ProfilerPanel,
    services::EditorServices,
    viewport::{
        render_viewport_toolbar_overlay, CameraControlMode, EditorCamera, GizmoHandler,
        GizmoInteractionResult, ToolMode, ViewportSettings,
    },
    CommandHistory, HierarchyPanel, InspectorPanel, Selection,
};
#[cfg(not(feature = "crusty"))]
use super::{
    console::{LogLevel, LogMessage},
    console_cmd::CommandContext,
};
use crate::engine::ecs::components::Transform;
use crate::engine::ecs::resources::PlayMode;
use egui::{Color32, RichText, Ui};
use egui_dock::tab_viewer::OnCloseResponse;
use egui_dock::TabViewer;
use hecs::World;

/// Context passed to tab viewer for rendering panels
pub struct EditorContext<'a> {
    pub services: &'a mut EditorServices,
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
    /// Input settings panel
    pub input_settings_panel: &'a mut super::InputSettingsPanel,
    /// InputActionSet snapshot for input settings display (read-only)
    pub action_set_snapshot: Option<&'a crate::engine::input::enhanced_action::InputActionSet>,
    /// Current play mode for viewport indicator
    pub play_mode: PlayMode,
    /// Output: play-mode action from viewport tab bar (set during render)
    pub toolbar_action: &'a mut MenuAction,
    /// Open mesh editors (keyed by content-relative mesh path)
    pub mesh_editors: &'a mut std::collections::HashMap<String, super::mesh_editor::MeshEditorData>,
    /// Open input action editor states (split from InputActionEditor to avoid borrow conflicts)
    pub ia_open_actions: &'a mut std::collections::HashMap<String, InputActionEditorState>,
    /// Open input context editor states (split from InputContextEditor)
    pub ic_open_contexts: &'a mut std::collections::HashMap<String, InputContextEditorState>,
    /// Available action names for mapping context editors
    pub ic_available_actions: &'a [String],
    /// Output: tab to undock to OS window (set by context menu or drag-to-undock, processed after DockArea::show)
    pub undock_request: &'a mut Option<EditorTab>,
    /// The screen rect of the main dock area, used for drag-to-undock detection.
    pub dock_area_rect: egui::Rect,
    /// Display name of the active scene, shown on the Viewport tab.
    pub current_scene_name: &'a str,
    /// Whether the active scene has unsaved changes (drives `* ` prefix on tab title).
    pub active_dirty: bool,
    /// Currently-active scene id.
    pub active_scene_id: super::scene_tab::SceneId,
    /// Read-only view of dormant scenes (for tab title lookup of inactive scenes).
    pub dormant_scenes: &'a [super::scene_tab::DormantScene],
    /// Output: scene id whose viewport tab the user closed (handled after DockArea::show).
    pub close_scene_request: &'a mut Option<super::scene_tab::SceneId>,
    /// Ordered list of viewport tab display names in the leaf that contains the active
    /// viewport, used to position the "+" New Scene button after the last tab.
    pub viewport_tab_labels: &'a [String],
    /// Output: the Console tab's content rect in egui points when it was
    /// visible this frame. The crusty-gui layout pass renders the ported
    /// console panel into this rect instead of the egui version.
    #[cfg(feature = "crusty")]
    pub crusty_console_rect: &'a mut Option<egui::Rect>,
    /// Output: the Hierarchy tab's content rect in egui points when it was
    /// visible this frame (same mechanism as `crusty_console_rect`).
    #[cfg(feature = "crusty")]
    pub crusty_hierarchy_rect: &'a mut Option<egui::Rect>,
    /// Output: the Inspector tab's content rect in egui points when it was
    /// visible this frame (same mechanism as `crusty_console_rect`).
    #[cfg(feature = "crusty")]
    pub crusty_inspector_rect: &'a mut Option<egui::Rect>,
    /// Output: the Asset Browser tab's content rect in egui points when it was
    /// visible this frame (same mechanism as `crusty_console_rect`).
    #[cfg(feature = "crusty")]
    pub crusty_asset_browser_rect: &'a mut Option<egui::Rect>,
    /// Output: the Profiler tab's content rect in egui points when it was
    /// visible this frame (same mechanism as `crusty_console_rect`).
    #[cfg(feature = "crusty")]
    pub crusty_profiler_rect: &'a mut Option<egui::Rect>,
}

/// Tab viewer that renders each panel type
pub struct EditorTabViewer<'a> {
    pub editor: EditorContext<'a>,
}

impl<'a> TabViewer for EditorTabViewer<'a> {
    type Tab = EditorTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            EditorTab::Viewport(id) => {
                let (name, dirty) = if *id == self.editor.active_scene_id {
                    let n = if self.editor.current_scene_name.is_empty() {
                        "Untitled Scene"
                    } else {
                        self.editor.current_scene_name
                    };
                    (n.to_string(), self.editor.active_dirty)
                } else if let Some(d) = self.editor.dormant_scenes.iter().find(|d| d.id == *id) {
                    let n = if d.display_name.is_empty() {
                        "Untitled Scene"
                    } else {
                        d.display_name.as_str()
                    };
                    (n.to_string(), d.dirty)
                } else {
                    ("(missing scene)".to_string(), false)
                };
                let prefix = if dirty { "* " } else { "" };
                format!("{}{}", prefix, name).into()
            }
            _ => tab.title_string().into(),
        }
    }

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(tab.id_string())
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
        match tab {
            EditorTab::Viewport(id) => {
                if *id == self.editor.active_scene_id {
                    self.render_viewport(ui);
                } else {
                    // Inactive viewport tabs render a placeholder; the world is parked.
                    // Clicking this tab triggers an active-scene swap (see app.rs).
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new("Switching scene...").weak());
                    });
                }
            }
            EditorTab::Hierarchy => self.render_hierarchy(ui),
            EditorTab::Inspector => self.render_inspector(ui),
            EditorTab::AssetBrowser => self.render_asset_browser(ui),
            EditorTab::Console => self.render_console(ui),
            EditorTab::Profiler => self.render_profiler(ui),
            EditorTab::InputSettings => self.render_input_settings(ui),
            EditorTab::MeshEditor(key) => self.render_mesh_editor_tab(ui, key.clone()),
            EditorTab::InputActionEditor(key) => self.render_input_action_tab(ui, key.clone()),
            EditorTab::InputContextEditor(key) => self.render_input_context_tab(ui, key.clone()),
        }
    }

    fn context_menu(
        &mut self,
        ui: &mut Ui,
        tab: &mut Self::Tab,
        _surface: egui_dock::SurfaceIndex,
        _node: egui_dock::NodeIndex,
    ) {
        // Viewport tabs cannot be undocked
        if !matches!(tab, EditorTab::Viewport(_))
            && ui.button("Open in New Window").clicked()
        {
            *self.editor.undock_request = Some(tab.clone());
            ui.close();
        }
    }

    fn closeable(&mut self, tab: &mut Self::Tab) -> bool {
        tab.closable()
    }

    fn on_close(&mut self, tab: &mut Self::Tab) -> OnCloseResponse {
        // Clean up editor state when per-file tabs are closed
        match tab {
            EditorTab::Viewport(id) => {
                // Defer the close to app.rs — it needs to handle dormant scene cleanup
                // and possibly prompt before discarding unsaved changes.
                *self.editor.close_scene_request = Some(*id);
            }
            EditorTab::MeshEditor(key) => {
                if let Some(data) = self.editor.mesh_editors.get_mut(key) {
                    data.open = false;
                }
            }
            EditorTab::InputActionEditor(key) => {
                if let Some(data) = self.editor.ia_open_actions.get_mut(key) {
                    data.open = false;
                }
            }
            EditorTab::InputContextEditor(key) => {
                if let Some(data) = self.editor.ic_open_contexts.get_mut(key) {
                    data.open = false;
                }
            }
            _ => {}
        }
        OnCloseResponse::Close
    }

    fn on_tab_button(&mut self, tab: &mut Self::Tab, response: &egui::Response) {
        if matches!(tab, EditorTab::Viewport(_)) {
            return;
        }

        // Visual hint while dragging outside dock area
        if response.dragged() {
            if let Some(pos) = response.ctx.pointer_interact_pos() {
                let dock_rect = self.editor.dock_area_rect.shrink(10.0);
                if !dock_rect.contains(pos) {
                    let painter = response.ctx.layer_painter(
                        egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("undock_hint")),
                    );
                    painter.text(
                        pos + egui::vec2(15.0, -10.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("Undock: {}", tab.title_string()),
                        egui::FontId::proportional(13.0),
                        Color32::from_rgb(180, 200, 255),
                    );
                }
            }
        }

        // Trigger undock when drag released outside dock area
        if response.drag_stopped() {
            if let Some(pos) = response.ctx.pointer_interact_pos() {
                if !self.editor.dock_area_rect.shrink(10.0).contains(pos) {
                    *self.editor.undock_request = Some(tab.clone());
                }
            }
        }
    }

    fn allowed_in_windows(&self, _tab: &mut Self::Tab) -> bool {
        false
    }
}

impl<'a> EditorTabViewer<'a> {
    /// Render a Godot-style "+" (New Scene) button on the viewport's tab bar,
    /// positioned right after the last viewport tab (LEFT side, not right).
    fn render_new_scene_button_on_tab_bar(
        &mut self,
        ctx: &egui::Context,
        viewport_content_rect: egui::Rect,
    ) {
        use super::menu_bar::MenuAction;

        // Match egui_dock's tab-width formula in widgets/dock_area/show/leaf.rs:
        //   text_width      = galley.x + 2 * x_spacing            (x_spacing = 8.0)
        //   close_btn_size  = min(TAB_CLOSE_BUTTON_SIZE, tab_bar.height)  (= 24, capped at 28)
        //   tab_width       = max(minimum_width, text_width + close_btn_size)
        const TAB_MIN_WIDTH: f32 = 120.0;
        const TAB_X_SPACING: f32 = 8.0;
        const TAB_CLOSE_BTN_SIZE: f32 = 24.0;
        const TAB_INTER_SPACING: f32 = 2.0;
        const TAB_BAR_HEIGHT: f32 = 28.0;
        const TAB_BODY_INNER_MARGIN_TOP: f32 = 4.0;

        // Use the same FontId egui_dock uses for tab labels (TextStyle::Button) so our
        // galley measurements match the actual rendered tab widths exactly.
        let button_font_id = ctx
            .style()
            .text_styles
            .get(&egui::TextStyle::Button)
            .cloned()
            .unwrap_or_else(|| egui::FontId::proportional(12.5));

        let close_btn = TAB_CLOSE_BTN_SIZE.min(TAB_BAR_HEIGHT);
        let mut total_tabs_width = 0.0_f32;
        let labels = self.editor.viewport_tab_labels;
        for (i, label) in labels.iter().enumerate() {
            let display = if label.is_empty() { "Untitled Scene" } else { label.as_str() };
            let galley = ctx.fonts_mut(|f| {
                f.layout_no_wrap(display.to_string(), button_font_id.clone(), Color32::WHITE)
            });
            let text_width = galley.size().x + 2.0 * TAB_X_SPACING;
            let tab_w = (text_width + close_btn).max(TAB_MIN_WIDTH);
            total_tabs_width += tab_w;
            if i + 1 < labels.len() {
                total_tabs_width += TAB_INTER_SPACING;
            }
        }
        if labels.is_empty() {
            total_tabs_width = TAB_MIN_WIDTH;
        }

        // Tab bar sits ABOVE viewport_content_rect, offset by inner_margin + tab_bar_height.
        let tab_bar_top = viewport_content_rect.top() - TAB_BODY_INNER_MARGIN_TOP - TAB_BAR_HEIGHT;
        let icon_size = 16.0_f32;
        let icon_top = tab_bar_top + (TAB_BAR_HEIGHT - icon_size) / 2.0;
        // 10 px padding between the last tab and the "+" icon.
        let icon_left = viewport_content_rect.left() + total_tabs_width + 10.0;

        // Resolve the loaded SVG texture for the Add icon.
        let texture_id = self
            .editor
            .icon_manager
            .and_then(|m| m.get(super::icons::ToolbarIcon::Add))
            .map(|t| t.id());

        egui::Area::new(egui::Id::new("viewport_new_scene_button"))
            .fixed_pos(egui::pos2(icon_left, icon_top))
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                // Frameless image button: only the icon itself highlights on hover/click,
                // never a surrounding box. Idle is dim grey, hover/active is full white.
                if let Some(tex) = texture_id {
                    // First reserve the rect so we can paint our own tinted image based on
                    // hover state (egui's ImageButton tint is fixed per call).
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(icon_size, icon_size),
                        egui::Sense::click(),
                    );
                    let tint = if response.is_pointer_button_down_on() {
                        Color32::from_gray(220)
                    } else if response.hovered() {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(170)
                    };
                    egui::Image::new(egui::load::SizedTexture::new(
                        tex,
                        egui::vec2(icon_size, icon_size),
                    ))
                    .tint(tint)
                    .paint_at(ui, rect);
                    let response = response.on_hover_text("New Scene");
                    if response.clicked() {
                        *self.editor.toolbar_action = MenuAction::NewScene;
                    }
                } else {
                    // Fallback: plain "+" glyph if the icon failed to load.
                    let response = ui.add(
                        egui::Button::new(
                            egui::RichText::new("+")
                                .size(16.0)
                                .color(Color32::from_gray(170)),
                        )
                        .frame(false),
                    );
                    if response.clicked() {
                        *self.editor.toolbar_action = MenuAction::NewScene;
                    }
                }
            });
    }


    /// Render the 3D viewport with the rendered scene texture
    fn render_viewport(&mut self, ui: &mut Ui) {
        // Get available size for the viewport (full panel area now, no toolbar taking space)
        let available_size = ui.available_size();

        // Track the desired viewport size (for texture resizing).
        // IMPORTANT: render to PHYSICAL pixels, not logical. `available_size` is in
        // logical units (egui's DPI-independent space). If we size the framebuffer in
        // logical units and then egui paints it onto a physical-pixel surface
        // (logical * pixels_per_point), the result is bilinearly upscaled and produces
        // visible banding/sampling artifacts on meshes and shadows during resize.
        //
        // We `floor` (not round) here to match egui's own rounding when it converts
        // logical rects to physical pixel rects in its rasterizer, eliminating
        // off-by-one sub-pixel mismatches that would otherwise produce edge artifacts.
        let ppp = ui.ctx().pixels_per_point();
        let new_width = (available_size.x.max(1.0) * ppp).floor() as u32;
        let new_height = (available_size.y.max(1.0) * ppp).floor() as u32;
        *self.editor.viewport_size = (new_width, new_height);

        // If we have a viewport texture, display it
        // Use the response rect for gizmo positioning (not available_rect_before_wrap)
        let mut viewport_rect = ui.available_rect_before_wrap();

        // Render the "+" New Scene button just after the last viewport tab.
        self.render_new_scene_button_on_tab_bar(ui.ctx(), viewport_rect);

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
            // Display the rendered scene texture filling the available space.
            // Use the texture's EXACT physical size converted back to logical units so
            // the egui Image quad matches the texture pixel-for-pixel. This guarantees
            // 1:1 sampling — no scaling, no banding/blurring during/after resize.
            let display_size_logical = egui::vec2(
                new_width as f32 / ppp,
                new_height as f32 / ppp,
            );
            let response = ui.add(
                egui::Image::new(egui::load::SizedTexture::new(
                    texture_id,
                    display_size_logical,
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

    /// Render the hierarchy panel (crusty port).
    ///
    /// The egui tab only records its content rect (the crusty layout pass
    /// in `game_client` draws the hierarchy there) and claims the space so
    /// the dock still sizes the tab normally.
    #[cfg(feature = "crusty")]
    fn render_hierarchy(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        *self.editor.crusty_hierarchy_rect = Some(rect);
        ui.allocate_rect(rect, egui::Sense::hover());
    }

    /// Render the hierarchy panel (egui fallback).
    #[cfg(not(feature = "crusty"))]
    fn render_hierarchy(&mut self, ui: &mut Ui) {
        self.editor.hierarchy_panel.show_contents(
            ui,
            self.editor.world,
            self.editor.selection,
            self.editor.play_mode,
        );
    }

    /// Render the inspector panel (crusty port).
    ///
    /// The egui tab only records its content rect (the crusty layout pass
    /// in `game_client` draws the inspector there) and claims the space so
    /// the dock still sizes the tab normally.
    #[cfg(feature = "crusty")]
    fn render_inspector(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        *self.editor.crusty_inspector_rect = Some(rect);
        ui.allocate_rect(rect, egui::Sense::hover());
    }

    /// Render the inspector panel (egui fallback).
    #[cfg(not(feature = "crusty"))]
    fn render_inspector(&mut self, ui: &mut Ui) {
        self.editor.inspector_panel.show_contents(
            ui,
            self.editor.world,
            self.editor.selection,
            self.editor.play_mode,
            self.editor.asset_browser,
        );
    }

    /// Render the asset browser panel.
    ///
    /// With the `crusty` feature the panel is ported to crusty-gui: the
    /// egui tab only records its content rect (same mechanism as the
    /// console) and claims the space so the dock sizes the tab normally.
    #[cfg(feature = "crusty")]
    fn render_asset_browser(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        *self.editor.crusty_asset_browser_rect = Some(rect);
        ui.allocate_rect(rect, egui::Sense::hover());
    }

    /// Render the asset browser panel
    #[cfg(not(feature = "crusty"))]
    fn render_asset_browser(&mut self, ui: &mut Ui) {
        self.editor.asset_browser.show(ui, self.editor.icon_manager);
    }

    /// Render the console panel.
    ///
    /// With the `crusty` feature the panel is ported to crusty-gui: the
    /// egui tab only records its content rect (the crusty layout pass in
    /// `game_client` draws the console there) and claims the space so the
    /// dock still sizes the tab normally.
    #[cfg(feature = "crusty")]
    fn render_console(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        *self.editor.crusty_console_rect = Some(rect);
        ui.allocate_rect(rect, egui::Sense::hover());
    }

    /// Render the console panel (egui fallback).
    #[cfg(not(feature = "crusty"))]
    fn render_console(&mut self, ui: &mut Ui) {
        let (info_count, warn_count, error_count) = self.editor.console_messages.counts();

        // Header with filter toggles (custom styled buttons instead of selectable_label)
        ui.horizontal(|ui| {
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

    /// Render the profiler panel.
    ///
    /// With the `crusty` feature the panel is ported to crusty-gui: the
    /// egui tab only records its content rect (same mechanism as the
    /// console) and claims the space so the dock sizes the tab normally.
    #[cfg(feature = "crusty")]
    fn render_profiler(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        *self.editor.crusty_profiler_rect = Some(rect);
        ui.allocate_rect(rect, egui::Sense::hover());
    }

    /// Render the profiler panel (egui fallback).
    #[cfg(not(feature = "crusty"))]
    fn render_profiler(&mut self, ui: &mut Ui) {
        self.editor.profiler_panel.show_contents(ui);
    }

    /// Render the input settings panel
    fn render_input_settings(&mut self, ui: &mut Ui) {
        self.editor
            .input_settings_panel
            .show_contents(ui, self.editor.action_set_snapshot);
    }

    /// Render a docked mesh editor tab
    fn render_mesh_editor_tab(&mut self, ui: &mut Ui, key: String) {
        if let Some(data) = self.editor.mesh_editors.get_mut(&key) {
            super::mesh_editor::MeshEditorPanel::show(ui, data, self.editor.asset_browser);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Mesh editor data not found").weak());
            });
        }
    }

    /// Render a docked input action editor tab
    fn render_input_action_tab(&mut self, ui: &mut Ui, key: String) {
        if let Some(data) = self.editor.ia_open_actions.get_mut(&key) {
            InputActionEditor::show_ui(ui, data);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Input action editor data not found").weak());
            });
        }
    }

    /// Render a docked mapping context editor tab
    fn render_input_context_tab(&mut self, ui: &mut Ui, key: String) {
        if let Some(data) = self.editor.ic_open_contexts.get_mut(&key) {
            InputContextEditor::show_ui(ui, data, self.editor.ic_available_actions);
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Mapping context editor data not found").weak());
            });
        }
    }
}
