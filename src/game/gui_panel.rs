//! GUI panels for engine stats and profiler
//!
//! Uses puffin for profiling data (custom egui integration coming later).

use egui;
use rust_engine::GameLoop;
use rust_engine::Renderer;

/// Render the engine stats panel
pub fn render_engine_stats(
    ui: &mut egui::Ui,
    entity_count: usize,
    game_loop: &GameLoop,
    camera_distance: f32,
    renderer: &Renderer,
) {
    ui.heading("Rust Game Engine");
    ui.separator();

    ui.label(format!("Entities: {}", entity_count));
    ui.label(format!("FPS: {:.1}", game_loop.fps()));
    ui.label(format!("Camera Distance: {:.1}", camera_distance));

    ui.separator();
    ui.heading("Camera Position (Z-up)");
    // Camera uses Y-up internally, convert to Z-up for display
    use rust_engine::engine::utils::coords::convert_position_yup_to_zup;
    let pos_zup = convert_position_yup_to_zup(renderer.camera_3d.position);
    ui.label(format!("X: {:.2}", pos_zup.x));  // Z-up forward
    ui.label(format!("Y: {:.2}", pos_zup.y));  // Z-up right
    ui.label(format!("Z: {:.2}", pos_zup.z));  // Z-up up - changes with Space/Shift

    ui.separator();
    ui.heading("Controls");
    ui.label("  WASD - Move camera");
    ui.label("  Space/Shift - Up/Down");
    ui.label("  Arrow Keys - Look around");
    ui.label("  Mouse Wheel - Zoom");
    ui.label("  0-5 - Debug views");
    ui.label("  R - Reload assets");
    ui.label("  C - Cache stats");
    ui.label("  F12 - Profiler");
    ui.label("  Ctrl+S - Save scene");
    ui.label("  ESC - Quit");
}

/// Render the profiler panel placeholder
///
/// TODO: Implement custom puffin integration for egui 0.33
pub fn render_profiler_panel(ui: &mut egui::Ui) {
    ui.heading("Profiler");
    ui.separator();

    // Note: puffin 0.19 API has changed significantly
    // The detailed frame data API requires more complex setup
    // For now, just show that profiling is enabled
    ui.label("Profiling is enabled.");
    ui.label("Use puffin::profile_scope!() to instrument code.");
    ui.separator();
    ui.label(egui::RichText::new("Note: Full profiler UI coming soon.").weak());
    ui.label("For detailed profiling, use puffin_viewer.");
}

/// Create the main stats window
pub fn create_stats_window(
    ctx: &egui::Context,
    entity_count: usize,
    game_loop: &GameLoop,
    camera_distance: f32,
    renderer: &Renderer,
) {
    egui::Window::new("Engine Stats")
        .default_pos([10.0, 10.0])
        .default_size([300.0, 400.0])
        .resizable(true)
        .vscroll(true)
        .show(ctx, |ui| {
            render_engine_stats(ui, entity_count, game_loop, camera_distance, renderer);
        });
}
