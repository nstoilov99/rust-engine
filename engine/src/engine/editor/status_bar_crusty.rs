//! Status bar rendered with crusty-gui (Phase 16 port).
//!
//! Reads the same `StatusBarState` as the egui version in `status_bar.rs`;
//! with the `crusty` feature enabled the egui bottom panel only reserves the
//! strip (and paints the panel background) and this draws the text there.

use crusty_gui::context::Ui;
use crusty_gui::math::{Color, Pos2, Rect};

use super::status_bar::StatusBarState;

pub fn status_bar_panel(ui: &mut Ui, bar_rect: egui::Rect, ppp: f32, state: &StatusBarState) {
    let rect = Rect::from_min_max(
        Pos2::new(bar_rect.min.x * ppp, bar_rect.min.y * ppp),
        Pos2::new(bar_rect.max.x * ppp, bar_rect.max.y * ppp),
    );
    let font = ui.style().fonts.body;
    // egui's `weak` text color on the dark theme.
    let weak = Color::from_srgb_u8(140, 140, 140, 255);
    let pad = 8.0 * ppp;

    let left_size = ui.text_mut().measure(&state.left_text, font, None);
    let y = rect.min.y + (rect.height() - left_size.y) * 0.5;
    ui.painter().text(
        Pos2::new(rect.min.x + pad, y),
        &state.left_text,
        font,
        weak,
        None,
    );

    if !state.right_text.is_empty() {
        let right_size = ui.text_mut().measure(&state.right_text, font, None);
        ui.painter().text(
            Pos2::new(rect.max.x - pad - right_size.x, y),
            &state.right_text,
            font,
            weak,
            None,
        );
    }
}
