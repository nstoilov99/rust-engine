//! Profiler toolbar rendering
//!
//! Renders the toolbar with pause/resume, clear, and frame statistics.

use egui::{Color32, RichText, Ui};

use super::data::ProfilerState;

/// Render the profiler toolbar
pub fn render(ui: &mut Ui, state: &mut ProfilerState) {
    crate::profile_scope!("profiler_toolbar");

    // First row: controls and current frame stats
    ui.horizontal(|ui| {
        // Pause/Resume button
        let button_text = if state.is_paused { "Resume" } else { "Pause" };
        let button_color = if state.is_paused {
            Color32::from_rgb(80, 180, 80) // Green for resume
        } else {
            Color32::from_rgb(180, 140, 60) // Orange/amber for pause
        };

        if ui
            .add(
                egui::Button::new(RichText::new(button_text).color(Color32::WHITE))
                    .fill(button_color)
                    .corner_radius(3.0),
            )
            .on_hover_text("Toggle capture (Space)")
            .clicked()
        {
            state.toggle_pause();
        }

        // Clear button
        if ui
            .add(
                egui::Button::new("Clear")
                    .fill(Color32::from_gray(60))
                    .corner_radius(3.0),
            )
            .on_hover_text("Clear frame history")
            .clicked()
        {
            state.clear();
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // Tracy button
        #[cfg(feature = "tracy")]
        {
            let (text, button_fill, tip) = if state.tracy_state.is_connected() {
                (
                    "Tracy ●",
                    Color32::from_rgb(50, 100, 50), // Dark green
                    "Tracy connected - capturing data",
                )
            } else if state.tracy_state.is_enabled() {
                (
                    "Tracy ○",
                    Color32::from_rgb(100, 80, 30), // Dark yellow/amber
                    "Tracy enabled - waiting for connection...\nLaunch Tracy GUI to connect",
                )
            } else {
                (
                    "Tracy",
                    Color32::from_gray(50),
                    "Enable Tracy profiler for deep analysis",
                )
            };

            if ui
                .add(
                    egui::Button::new(RichText::new(text).color(Color32::WHITE))
                        .fill(button_fill)
                        .corner_radius(3.0),
                )
                .on_hover_text(tip)
                .clicked()
            {
                state.show_tracy_popup = !state.show_tracy_popup;
            }
        }

        #[cfg(not(feature = "tracy"))]
        {
            ui.add_enabled(
                false,
                egui::Button::new(RichText::new("Tracy").color(Color32::from_gray(80)))
                    .fill(Color32::from_gray(35))
                    .corner_radius(3.0),
            )
            .on_disabled_hover_text(
                "Tracy not available.\nBuild with: cargo build --features tracy",
            );
        }

        ui.add_space(4.0);

        // Paused indicator
        if state.is_paused {
            ui.label(
                RichText::new("PAUSED")
                    .color(Color32::from_rgb(255, 220, 80))
                    .strong(),
            );
            ui.add_space(8.0);
        }

        ui.separator();
        ui.add_space(4.0);

        // Frame statistics
        if let Some(frame) = state.selected_frame() {
            // Frame number
            ui.label(
                RichText::new(format!("Frame #{}", frame.frame_number))
                    .color(Color32::from_gray(200)),
            );

            ui.separator();

            // Frame duration
            let duration_ms = frame.duration_ms();
            let fps = 1000.0 / duration_ms;
            let duration_color = if duration_ms < 16.67 {
                Color32::from_rgb(80, 200, 80) // Green
            } else if duration_ms < 33.33 {
                Color32::from_rgb(220, 180, 60) // Yellow
            } else {
                Color32::from_rgb(220, 80, 80) // Red
            };

            ui.label(
                RichText::new(format!("{:.2} ms", duration_ms))
                    .color(duration_color)
                    .strong(),
            );

            ui.separator();

            ui.label(RichText::new(format!("{:.0} FPS", fps)).color(duration_color));

            ui.separator();

            // Thread count
            ui.label(
                RichText::new(format!("{} threads", frame.thread_count()))
                    .color(Color32::from_gray(180)),
            );

            ui.separator();

            // Scope count
            ui.label(
                RichText::new(format!("{} scopes", frame.total_scopes))
                    .color(Color32::from_gray(180)),
            );

            ui.separator();

            // Data size
            let size_kb = frame.data_size_bytes as f64 / 1024.0;
            ui.label(RichText::new(format!("{:.1} KB", size_kb)).color(Color32::from_gray(160)));
        } else {
            ui.label(RichText::new("No frame selected").weak());
        }

        // Right-aligned buttons
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Settings button
            if ui
                .add(egui::Button::new("Settings").corner_radius(3.0))
                .on_hover_text("Profiler settings")
                .clicked()
            {
                state.show_settings = !state.show_settings;
            }

            // Info button
            if ui
                .add(egui::Button::new("Info").corner_radius(3.0))
                .on_hover_text("Show controls help")
                .clicked()
            {
                state.show_info = !state.show_info;
            }

            ui.add_space(4.0);
        });
    });

    // Second row: aggregate statistics (avg/min/max)
    let stats = state.frame_statistics();
    if stats.frame_count > 0 {
        ui.horizontal(|ui| {
            ui.add_space(8.0);

            // Color based on average FPS
            let avg_color = if stats.avg_fps >= 60.0 {
                Color32::from_rgb(80, 200, 80) // Green
            } else if stats.avg_fps >= 30.0 {
                Color32::from_rgb(220, 180, 60) // Yellow
            } else {
                Color32::from_rgb(220, 80, 80) // Red
            };

            ui.label(RichText::new("Avg:").color(Color32::from_gray(140)));
            ui.label(
                RichText::new(format!("{:.2} ms", stats.avg_duration_ms))
                    .color(avg_color)
                    .strong(),
            );
            ui.label(RichText::new(format!("({:.0} FPS)", stats.avg_fps)).color(avg_color));

            ui.separator();

            ui.label(RichText::new("Min:").color(Color32::from_gray(140)));
            ui.label(
                RichText::new(format!("{:.2} ms", stats.min_duration_ms))
                    .color(Color32::from_gray(180)),
            );

            ui.separator();

            ui.label(RichText::new("Max:").color(Color32::from_gray(140)));
            ui.label(
                RichText::new(format!("{:.2} ms", stats.max_duration_ms))
                    .color(Color32::from_gray(180)),
            );

            ui.separator();

            ui.label(
                RichText::new(format!("({} frames)", stats.frame_count))
                    .color(Color32::from_gray(120)),
            );
        });
    }
}

/// Render the settings popup
pub fn render_settings_popup(ui: &mut Ui, state: &mut ProfilerState) {
    if !state.show_settings {
        return;
    }

    egui::Window::new("Profiler Settings")
        .collapsible(false)
        .resizable(false)
        .default_width(320.0)
        .show(ui.ctx(), |ui| {
            // Frame History Section
            ui.heading("Frame History");
            egui::Frame::new()
                .fill(Color32::from_gray(35))
                .inner_margin(8.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Capacity:");
                        let mut capacity = state.settings.history_capacity;
                        if ui
                            .add(
                                egui::DragValue::new(&mut capacity)
                                    .range(100..=10000)
                                    .speed(50)
                                    .suffix(" frames"),
                            )
                            .changed()
                        {
                            state.resize_history(capacity);
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Max bars:");
                        ui.add(
                            egui::DragValue::new(&mut state.settings.max_display_bars)
                                .range(50..=500)
                                .speed(10),
                        );
                    });
                });

            ui.add_space(8.0);

            // Performance Thresholds Section
            ui.heading("Performance Thresholds");
            ui.label(
                RichText::new("Affects bar colors + flamegraph")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            egui::Frame::new()
                .fill(Color32::from_gray(35))
                .inner_margin(8.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    // FPS Presets
                    ui.horizontal(|ui| {
                        ui.label("Presets:");
                        if ui.button("60 FPS").clicked() {
                            state.settings.fast_threshold_ms = 1.0;
                            state.settings.warning_threshold_ms = 5.0;
                            state.settings.slow_threshold_ms = 16.67;
                        }
                        if ui.button("30 FPS").clicked() {
                            state.settings.fast_threshold_ms = 5.0;
                            state.settings.warning_threshold_ms = 16.0;
                            state.settings.slow_threshold_ms = 33.33;
                        }
                        if ui.button("144 FPS").clicked() {
                            state.settings.fast_threshold_ms = 0.5;
                            state.settings.warning_threshold_ms = 2.0;
                            state.settings.slow_threshold_ms = 6.9;
                        }
                    });

                    ui.add_space(4.0);

                    // Custom thresholds with color indicators
                    ui.horizontal(|ui| {
                        // Green color indicator
                        let (rect, _) = ui
                            .allocate_exact_size(egui::Vec2::new(12.0, 12.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 2.0, Color32::from_rgb(80, 200, 80));

                        ui.label("Fast <");
                        ui.add(
                            egui::DragValue::new(&mut state.settings.fast_threshold_ms)
                                .speed(0.1)
                                .range(0.1..=state.settings.warning_threshold_ms - 0.1)
                                .suffix(" ms"),
                        );
                    });

                    ui.horizontal(|ui| {
                        // Yellow color indicator
                        let (rect, _) = ui
                            .allocate_exact_size(egui::Vec2::new(12.0, 12.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 2.0, Color32::from_rgb(220, 180, 60));

                        ui.label("Warning <");
                        ui.add(
                            egui::DragValue::new(&mut state.settings.warning_threshold_ms)
                                .speed(0.5)
                                .range(
                                    state.settings.fast_threshold_ms + 0.1
                                        ..=state.settings.slow_threshold_ms - 0.1,
                                )
                                .suffix(" ms"),
                        );
                    });

                    ui.horizontal(|ui| {
                        // Red color indicator
                        let (rect, _) = ui
                            .allocate_exact_size(egui::Vec2::new(12.0, 12.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 2.0, Color32::from_rgb(220, 80, 80));

                        ui.label("Slow >=");
                        ui.add(
                            egui::DragValue::new(&mut state.settings.slow_threshold_ms)
                                .speed(1.0)
                                .range(state.settings.warning_threshold_ms + 0.1..=100.0)
                                .suffix(" ms"),
                        );
                    });
                });

            ui.add_space(8.0);

            // Flamegraph Grid Section
            ui.heading("Flamegraph Grid");
            egui::Frame::new()
                .fill(Color32::from_gray(35))
                .inner_margin(8.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Major grid:");
                        egui::ComboBox::from_id_salt("grid_spacing")
                            .selected_text(format!("{} ms", state.settings.grid_spacing_ms))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut state.settings.grid_spacing_ms,
                                    0.1,
                                    "0.1 ms",
                                );
                                ui.selectable_value(
                                    &mut state.settings.grid_spacing_ms,
                                    0.5,
                                    "0.5 ms",
                                );
                                ui.selectable_value(
                                    &mut state.settings.grid_spacing_ms,
                                    1.0,
                                    "1 ms",
                                );
                                ui.selectable_value(
                                    &mut state.settings.grid_spacing_ms,
                                    2.0,
                                    "2 ms",
                                );
                                ui.selectable_value(
                                    &mut state.settings.grid_spacing_ms,
                                    5.0,
                                    "5 ms",
                                );
                                ui.selectable_value(
                                    &mut state.settings.grid_spacing_ms,
                                    10.0,
                                    "10 ms",
                                );
                            });

                        ui.label("Sub-div:");
                        egui::ComboBox::from_id_salt("sub_grid")
                            .selected_text(if state.settings.sub_grid_divisions == 0 {
                                "None".to_string()
                            } else {
                                state.settings.sub_grid_divisions.to_string()
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut state.settings.sub_grid_divisions,
                                    0,
                                    "None",
                                );
                                ui.selectable_value(&mut state.settings.sub_grid_divisions, 2, "2");
                                ui.selectable_value(&mut state.settings.sub_grid_divisions, 4, "4");
                                ui.selectable_value(&mut state.settings.sub_grid_divisions, 5, "5");
                                ui.selectable_value(
                                    &mut state.settings.sub_grid_divisions,
                                    10,
                                    "10",
                                );
                            });
                    });
                });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if ui.button("Reset to Defaults").clicked() {
                    let default_settings = super::data::ProfilerSettings::default();
                    state.resize_history(default_settings.history_capacity);
                    state.settings = default_settings;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        state.show_settings = false;
                    }
                });
            });
        });
}

/// Render the info/help popup
pub fn render_info_popup(ui: &mut Ui, state: &mut ProfilerState) {
    if !state.show_info {
        return;
    }

    egui::Window::new("Profiler Controls")
        .collapsible(false)
        .resizable(false)
        .default_width(280.0)
        .show(ui.ctx(), |ui| {
            ui.heading("Keyboard Shortcuts");
            ui.label("Space - Pause/Resume capture");
            ui.label("F12 - Toggle profiler visibility");

            ui.add_space(10.0);
            ui.heading("Flamegraph Controls");
            ui.label("Drag - Pan view");
            ui.label("Ctrl + Scroll - Zoom in/out");
            ui.label("Click scope - Zoom to fit");
            ui.label("Double-click - Reset view");

            ui.add_space(10.0);
            ui.heading("Frame History");
            ui.label("Click bar - Select frame & pause");

            ui.add_space(10.0);
            ui.heading("Table View");
            ui.label("Click header - Sort by column");
            ui.label("Type in filter - Filter scopes");

            ui.add_space(10.0);
            if ui.button("Close").clicked() {
                state.show_info = false;
            }
        });
}

/// Render the Tracy profiler popup
pub fn render_tracy_popup(ui: &mut Ui, state: &mut ProfilerState) {
    if !state.show_tracy_popup {
        return;
    }

    egui::Window::new("Tracy Profiler")
        .collapsible(false)
        .resizable(false)
        .default_width(320.0)
        .show(ui.ctx(), |ui| {
            // Status section
            ui.heading("Status");
            egui::Frame::new()
                .fill(Color32::from_gray(35))
                .inner_margin(8.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    #[cfg(feature = "tracy")]
                    {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("● Tracy Active")
                                    .color(Color32::from_rgb(80, 200, 80))
                                    .strong(),
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Connect Tracy GUI to capture data")
                                .small()
                                .color(Color32::from_gray(160)),
                        );
                        ui.label(
                            RichText::new("Tracy v0.11.1 recommended")
                                .small()
                                .color(Color32::from_gray(120)),
                        );
                    }

                    #[cfg(not(feature = "tracy"))]
                    {
                        ui.label(
                            RichText::new("Tracy not compiled in").color(Color32::from_gray(140)),
                        );
                        ui.label(
                            RichText::new("Build with: cargo build --features tracy")
                                .small()
                                .color(Color32::from_gray(100)),
                        );
                    }
                });

            ui.add_space(8.0);

            // Info section
            ui.heading("About Tracy");
            egui::Frame::new()
                .fill(Color32::from_gray(35))
                .inner_margin(8.0)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.label("Tracy provides deep profiling with:");
                    ui.label("  • Nanosecond precision timing");
                    ui.label("  • Memory allocation tracking");
                    ui.label("  • GPU profiling support");
                    ui.label("  • Lock contention analysis");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Puffin:")
                                .strong()
                                .color(Color32::from_gray(180)),
                        );
                        ui.label(
                            RichText::new("Quick in-app overview")
                                .small()
                                .color(Color32::from_gray(140)),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Tracy:")
                                .strong()
                                .color(Color32::from_gray(180)),
                        );
                        ui.label(
                            RichText::new("Deep investigation tool")
                                .small()
                                .color(Color32::from_gray(140)),
                        );
                    });
                });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    state.show_tracy_popup = false;
                }
            });
        });
}
