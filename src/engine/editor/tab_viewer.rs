//! TabViewer implementation for egui_dock
//!
//! Handles rendering of each tab type in the dock layout.

use super::{
    console::{LogFilter, LogLevel, LogMessage},
    dock_layout::EditorTab,
    profiler::ProfilerPanel,
    CommandHistory, HierarchyPanel, InspectorPanel, Selection,
};
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
    /// Console log messages
    pub console_messages: &'a [LogMessage],
    /// Console log filter settings
    pub log_filter: &'a mut LogFilter,
    /// Viewport texture ID for rendering the 3D scene
    pub viewport_texture_id: Option<egui::TextureId>,
    /// Current viewport size (for detecting resize)
    pub viewport_size: &'a mut (u32, u32),
    /// Profiler panel
    pub profiler_panel: &'a mut ProfilerPanel,
}

/// Tab viewer that renders each panel type
pub struct EditorTabViewer<'a> {
    pub editor: EditorContext<'a>,
}

impl<'a> TabViewer for EditorTabViewer<'a> {
    type Tab = EditorTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.title().into()
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
    /// Render the 3D viewport with the rendered scene texture
    fn render_viewport(&mut self, ui: &mut Ui) {
        let available_size = ui.available_size();

        // Track the desired viewport size (for texture resizing)
        let new_width = (available_size.x.max(1.0)) as u32;
        let new_height = (available_size.y.max(1.0)) as u32;
        *self.editor.viewport_size = (new_width, new_height);

        // If we have a viewport texture, display it
        if let Some(texture_id) = self.editor.viewport_texture_id {
            // Display the rendered scene texture filling the available space
            let image = egui::Image::new(egui::load::SizedTexture::new(
                texture_id,
                egui::vec2(available_size.x, available_size.y),
            ));
            ui.add(image);
        } else {
            // Fallback placeholder when texture isn't available
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(available_size.y / 2.0 - 40.0);
                    ui.heading("3D Viewport");
                    ui.label(format!("Size: {:.0} x {:.0}", available_size.x, available_size.y));
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Initializing viewport texture...").weak());
                });
            });
        }
    }

    /// Render the hierarchy panel
    fn render_hierarchy(&mut self, ui: &mut Ui) {
        self.editor
            .hierarchy_panel
            .show_contents(ui, self.editor.world, self.editor.selection);
    }

    /// Render the inspector panel
    fn render_inspector(&mut self, ui: &mut Ui) {
        self.editor
            .inspector_panel
            .show_contents(ui, self.editor.world, self.editor.selection);
    }

    /// Render the asset browser (placeholder)
    fn render_asset_browser(&mut self, ui: &mut Ui) {
        ui.heading("Asset Browser");
        ui.separator();
        ui.label("Drag assets from here into the viewport or hierarchy.");

        ui.add_space(10.0);

        // Placeholder grid of assets
        ui.horizontal_wrapped(|ui| {
            for i in 0..12 {
                ui.group(|ui| {
                    ui.set_min_size(egui::vec2(64.0, 80.0));
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("[ ]").size(24.0));
                        ui.small(format!("Asset {}", i));
                    });
                });
            }
        });

        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("(Asset browser will be fully implemented in a future tutorial)")
                .weak(),
        );
    }

    /// Render the console panel
    fn render_console(&mut self, ui: &mut Ui) {
        // Count messages by level
        let (info_count, warn_count, error_count) =
            LogFilter::count_by_level(self.editor.console_messages);

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
            let error_text = RichText::new(format!("Errors ({})", error_count))
                .color(if self.editor.log_filter.show_error {
                    LogLevel::Error.color()
                } else {
                    Color32::GRAY
                });
            if ui.add(egui::Button::new(error_text).fill(error_fill).corner_radius(3.0)).clicked() {
                self.editor.log_filter.show_error = !self.editor.log_filter.show_error;
            }

            // Warning filter button - dark orange when active
            let warn_fill = if self.editor.log_filter.show_warning {
                Color32::from_rgba_unmultiplied(100, 80, 40, 180)
            } else {
                Color32::from_gray(45)
            };
            let warn_text = RichText::new(format!("Warnings ({})", warn_count))
                .color(if self.editor.log_filter.show_warning {
                    LogLevel::Warning.color()
                } else {
                    Color32::GRAY
                });
            if ui.add(egui::Button::new(warn_text).fill(warn_fill).corner_radius(3.0)).clicked() {
                self.editor.log_filter.show_warning = !self.editor.log_filter.show_warning;
            }

            // Info filter button - dark blue-gray when active
            let info_fill = if self.editor.log_filter.show_info {
                Color32::from_rgba_unmultiplied(60, 70, 90, 180)
            } else {
                Color32::from_gray(45)
            };
            let info_text = RichText::new(format!("Info ({})", info_count))
                .color(if self.editor.log_filter.show_info {
                    LogLevel::Info.color()
                } else {
                    Color32::GRAY
                });
            if ui.add(egui::Button::new(info_text).fill(info_fill).corner_radius(3.0)).clicked() {
                self.editor.log_filter.show_info = !self.editor.log_filter.show_info;
            }
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
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
    }

    /// Render the profiler panel
    fn render_profiler(&mut self, ui: &mut Ui) {
        self.editor.profiler_panel.show_contents(ui);
    }
}
