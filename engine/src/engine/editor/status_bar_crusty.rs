//! Status bar rendered with crusty-gui.

use crusty_gui::context::Ui;
use crusty_gui::math::{Color, Pos2, Rect};

use super::status_bar::StatusBarState;

pub fn status_bar_panel(ui: &mut Ui, bar_rect: Rect, state: &StatusBarState) {
    let rect = bar_rect;
    let font = ui.style().fonts.body;
    // Weak text color on the dark theme.
    let weak = Color::from_srgb_u8(140, 140, 140, 255);
    let pad = 8.0;

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
