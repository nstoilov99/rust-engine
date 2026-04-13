//! Table view for profiler data
//!
//! Displays scope statistics in a sortable, filterable table.

use egui::{Color32, RichText, Ui};

use super::data::{ProfilerState, SortColumn, SortDirection};

/// Render the table view
pub fn render(ui: &mut Ui, state: &mut ProfilerState) {
    crate::profile_scope!("profiler_table_view");

    // Filter input
    ui.horizontal(|ui| {
        ui.label("Filter:");
        let response = ui.text_edit_singleline(&mut state.filter_text);
        if response.changed() {
            // Filter is applied in calculate_scope_stats
        }
        if ui.button("Clear").clicked() {
            state.filter_text.clear();
        }
    });

    ui.add_space(4.0);

    // Table header
    let header_height = 24.0;
    let available_width = ui.available_width();

    // Column widths (proportional)
    let col_location = available_width * 0.25;
    let col_name = available_width * 0.20;
    let col_calls = available_width * 0.10;
    let col_total = available_width * 0.15;
    let col_mean = available_width * 0.15;
    let col_max = available_width * 0.15;

    // Render header buttons FIRST (these may mutate sort state)
    egui::Grid::new("profiler_table_header")
        .num_columns(6)
        .min_col_width(40.0)
        .striped(false)
        .show(ui, |ui| {
            // Header row with sort buttons
            render_header_button(ui, "Location", SortColumn::Location, col_location, state);
            render_header_button(ui, "Name", SortColumn::Name, col_name, state);
            render_header_button(ui, "Calls", SortColumn::CallCount, col_calls, state);
            render_header_button(ui, "Total", SortColumn::TotalTime, col_total, state);
            render_header_button(ui, "Mean", SortColumn::MeanTime, col_mean, state);
            render_header_button(ui, "Max", SortColumn::MaxTime, col_max, state);
            ui.end_row();
        });

    ui.separator();

    // Copy state values needed before borrowing stats (to avoid borrow conflict)
    let settings = state.settings.clone();
    let filter_is_empty = state.filter_text.is_empty();

    // Get cached stats AFTER header rendering (since header may change sort)
    let stats = state.get_scope_stats();

    if stats.is_empty() {
        ui.centered_and_justified(|ui| {
            if filter_is_empty {
                ui.label(RichText::new("No scope data available").weak());
            } else {
                ui.label(RichText::new("No scopes match filter").weak());
            }
        });
        return;
    }

    // Table body with scroll
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("profiler_table_body")
                .num_columns(6)
                .min_col_width(40.0)
                .striped(true)
                .show(ui, |ui| {
                    for stat in stats {
                        // Location
                        ui.add_sized(
                            [col_location, header_height],
                            egui::Label::new(
                                RichText::new(stat.location.as_ref())
                                    .color(Color32::from_gray(160))
                                    .small(),
                            )
                            .truncate(),
                        );

                        // Name
                        ui.add_sized(
                            [col_name, header_height],
                            egui::Label::new(stat.name.as_ref()).truncate(),
                        );

                        // Call count
                        ui.add_sized(
                            [col_calls, header_height],
                            egui::Label::new(
                                RichText::new(format!("{}", stat.call_count))
                                    .color(Color32::from_gray(180)),
                            ),
                        );

                        // Total time
                        let total_color = time_color(stat.total_time_ms(), &settings);
                        ui.add_sized(
                            [col_total, header_height],
                            egui::Label::new(
                                RichText::new(format!("{:.3} ms", stat.total_time_ms()))
                                    .color(total_color),
                            ),
                        );

                        // Mean time
                        let mean_color = time_color(stat.mean_time_ms(), &settings);
                        ui.add_sized(
                            [col_mean, header_height],
                            egui::Label::new(
                                RichText::new(format!("{:.3} ms", stat.mean_time_ms()))
                                    .color(mean_color),
                            ),
                        );

                        // Max time
                        let max_color = time_color(stat.max_time_ms(), &settings);
                        ui.add_sized(
                            [col_max, header_height],
                            egui::Label::new(
                                RichText::new(format!("{:.3} ms", stat.max_time_ms()))
                                    .color(max_color),
                            ),
                        );

                        ui.end_row();
                    }
                });
        });
}

/// Render a sortable header button
fn render_header_button(
    ui: &mut Ui,
    label: &str,
    column: SortColumn,
    width: f32,
    state: &mut ProfilerState,
) {
    let is_sorted = state.sort_column == column;
    let arrow = if is_sorted {
        match state.sort_direction {
            SortDirection::Ascending => " ^",
            SortDirection::Descending => " v",
        }
    } else {
        ""
    };

    let text = format!("{}{}", label, arrow);
    let text_color = if is_sorted {
        Color32::WHITE
    } else {
        Color32::from_gray(200)
    };

    let response = ui.add_sized(
        [width, 20.0],
        egui::Button::new(RichText::new(text).color(text_color).strong())
            .fill(if is_sorted {
                Color32::from_gray(50)
            } else {
                Color32::from_gray(40)
            })
            .corner_radius(2.0),
    );

    if response.clicked() {
        if is_sorted {
            // Toggle direction
            state.sort_direction = match state.sort_direction {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
        } else {
            // Switch to this column, default descending
            state.sort_column = column;
            state.sort_direction = SortDirection::Descending;
        }
    }
}

/// Get color for a time value based on thresholds
fn time_color(time_ms: f64, settings: &super::data::ProfilerSettings) -> Color32 {
    if time_ms < settings.fast_threshold_ms as f64 {
        Color32::from_rgb(120, 200, 120) // Green
    } else if time_ms < settings.warning_threshold_ms as f64 {
        Color32::from_rgb(220, 200, 80) // Yellow
    } else if time_ms < settings.slow_threshold_ms as f64 {
        Color32::from_rgb(220, 140, 80) // Orange
    } else {
        Color32::from_rgb(220, 80, 80) // Red
    }
}
