//! Standalone in-game HUD, drawn with crusty-gui (M7 D7). Package 6 lands
//! the seam with just the connection chip; combat widgets follow in
//! package 7.

use rust_engine::engine::gui::crusty::{Color, Pos2, Rect, Ui, Vec2};

const MARGIN: f32 = 10.0;
const FONT_PX: f32 = 13.0;

/// Top-right chip showing the net status line (moved out of the window
/// title). No-op when there is no net session.
pub fn draw(ui: &mut Ui, net_status: Option<&str>) {
    let Some(status) = net_status else { return };
    let screen = ui.available();
    let pad = Vec2::new(8.0, 4.0);
    let mut p = ui.painter();
    let text_size = p.measure_text(status, FONT_PX, None);
    let size = Vec2::new(text_size.x + pad.x * 2.0, text_size.y + pad.y * 2.0);
    let min = Pos2::new(screen.max.x - MARGIN - size.x, screen.min.y + MARGIN);
    p.rect_filled(
        Rect::from_min_size(min, size),
        4.0,
        Color::rgba(0.0, 0.0, 0.0, 0.55),
    );
    p.text(
        Pos2::new(min.x + pad.x, min.y + pad.y),
        status,
        FONT_PX,
        Color::WHITE,
        None,
    );
}
